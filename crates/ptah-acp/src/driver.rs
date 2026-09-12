//! The per-session driver: the JSON-RPC connection loop serving the
//! command channel, prompt turns, and config-option folding; plus
//! typed-result injection and run-end teardown.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, ContentChunk, EnvVariable, McpServer, McpServerStdio,
    PromptRequest, PromptResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigId, SessionConfigKind,
    SessionConfigOption, SessionConfigOptionValue, SessionId, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, StopReason, TextContent,
};
use agent_client_protocol::{ByteStreams, Client, ConnectionTo};
use tokio::sync::{mpsc, oneshot};

use ptah_core::config::AgentSpec;
use ptah_core::events::{PlanEntry, PlanStatus, SessionEvent};
use ptah_core::groups::ProcessGroups;
use ptah_core::ports::{
    AgentTransport, BridgeConfig, EventSink, HeadlessPolicy, InteractionPolicy,
};
use ptah_core::session::{
    SessionCmd, SessionError, SessionHandle, SessionOptions, TurnError, TurnOutcome, UsageCounts,
};
use ptah_core::turn::{PeekInputs, TurnFold, status_string, submission_sink};
use ptah_result::{bind_result_socket, spawn_result_channel};

use super::process::{self, kill_and_reap};
use super::proto;

/// How long a timed-out prompt waits for the (cancelled) response before
/// raising the timeout error to the script anyway.
const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// Start one agent subprocess and drive it until closed.
pub async fn start_session(
    spec: &AgentSpec,
    opts: SessionOptions,
    sink: Arc<dyn EventSink>,
) -> Result<SessionHandle, SessionError> {
    start_session_inner(spec, opts, sink, None).await
}

/// The registry-tracking core of [`start_session`]: `groups` (when
/// present) registers the spawned agent's group-leader pid for the
/// composition root's second-signal sweep, and the driver deregisters
/// it when it disposes of the child.
async fn start_session_inner(
    spec: &AgentSpec,
    opts: SessionOptions,
    sink: Arc<dyn EventSink>,
    groups: Option<Arc<ProcessGroups>>,
) -> Result<SessionHandle, SessionError> {
    let proc = process::spawn(spec, &opts.label, sink.clone(), groups.as_ref())?;
    let process::AgentProcess {
        stdin,
        stdout,
        pid,
        child,
        stderr_task,
    } = proc;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SessionCmd>();
    let (ready_tx, ready_rx) = oneshot::channel::<Result<SessionId, SessionError>>();
    let (done_tx, done_rx) = tokio::sync::watch::channel(false);

    let fold = Arc::new(Mutex::new(TurnFold::with_cwd(opts.cwd.clone())));
    // Live config-option state (session/new → updates → sets), shared
    // with the driver connection and snapshotted by the handle.
    let config_options = Arc::new(Mutex::new(Vec::<SessionConfigOption>::new()));
    let label = opts.label.clone();

    // Typed result contract: bind the per-session channel and offer the
    // agent the bridge MCP server alongside its own servers. Failures to
    // set up the channel degrade (result stays nil), never fail the
    // session.
    let mut mcp_servers = opts.mcp_servers.clone();
    let mut result_channel: Option<ptah_result::ResultChannel> = None;
    if let Some(contract) = opts.result.clone() {
        match std::env::current_exe() {
            Ok(exe) => match bind_result_socket().await {
                Ok((listener, path)) => {
                    let (cancel_tx, _cancel_rx) = tokio::sync::watch::channel(false);
                    let channel = spawn_result_channel(
                        listener,
                        contract.clone(),
                        submission_sink(fold.clone()),
                        sink.clone(),
                        label.clone(),
                        cancel_tx,
                    );
                    sink.emit(
                        &label,
                        SessionEvent::Lifecycle {
                            message: format!(
                                "{label}: typed-result contract active (socket {})",
                                path.display()
                            ),
                        },
                    );
                    let bridge = BridgeConfig::ptah_bridge();
                    mcp_servers.push(McpServer::Stdio(
                        McpServerStdio::new(bridge.server_name, exe)
                            .args(vec!["__bridge".to_string()])
                            .env(vec![
                                EnvVariable::new(bridge.addr_env, path.display().to_string()),
                                EnvVariable::new(bridge.schema_env, contract.schema_json()),
                            ]),
                    ));
                    result_channel = Some(channel);
                }
                Err(e) => sink.emit(
                    &label,
                    SessionEvent::Lifecycle {
                        message: format!(
                            "{label}: typed results unavailable (cannot bind result socket: {e}); \
                             prompts will return result = nil"
                        ),
                    },
                ),
            },
            Err(e) => sink.emit(
                &label,
                SessionEvent::Lifecycle {
                    message: format!(
                        "{label}: typed results unavailable (cannot resolve ptah executable: {e}); \
                         prompts will return result = nil"
                    ),
                },
            ),
        }
    }

    let driver_label = label.clone();
    let teardown_label = label.clone();
    let driver_fold = fold.clone();
    let driver_sink = sink.clone();
    let driver_config = config_options.clone();
    let driver_groups = groups;

    // Headless permission posture for this session's agent→client
    // requests; the policy is the seam an interactive front end replaces.
    let policy: Arc<dyn InteractionPolicy> = Arc::new(HeadlessPolicy);

    let driver = tokio::spawn(async move {
        let child_guard = child;
        let stderr_task = stderr_task;

        let notif_fold = driver_fold.clone();
        let notif_sink = driver_sink.clone();
        let notif_label = driver_label.clone();
        let notif_config = driver_config.clone();
        let permission_policy = policy.clone();

        let builder = Client
            .builder()
            // ptah declares no client capabilities: agent-to-client
            // requests it has no support for (fs, terminal, elicitation,
            // …) are answered promptly with a method-not-found (-32601)
            // error so turns never hang. The one exception — ptah runs
            // headless and nobody is there to be asked — is
            // `session/request_permission`, which is passed through
            // (`retry: true`) to the allow-all handler below.
            .on_receive_dispatch(
                async move |dispatch: agent_client_protocol::Dispatch, _cx| match dispatch {
                    agent_client_protocol::Dispatch::Request(msg, responder) => {
                        if msg.method() == "session/request_permission" {
                            Ok(agent_client_protocol::Handled::No {
                                message: agent_client_protocol::Dispatch::Request(msg, responder),
                                retry: true,
                            })
                        } else {
                            responder.respond_with_error(
                                agent_client_protocol::Error::method_not_found(),
                            )?;
                            Ok(agent_client_protocol::Handled::Yes)
                        }
                    }
                    other => Ok(agent_client_protocol::Handled::No {
                        message: other,
                        retry: false,
                    }),
                },
                agent_client_protocol::on_receive_dispatch!(),
            )
            // The interaction policy answers permission requests (the
            // headless policy selects an allow option the agent offered —
            // see its docs). An offer with nothing selectable falls back
            // to method-not-found; every other request kind is answered
            // by the dispatch handler above.
            .on_receive_request(
                async move |req: RequestPermissionRequest,
                            responder: agent_client_protocol::Responder<
                    RequestPermissionResponse,
                >,
                            _cx| {
                    match permission_policy.select_permission(&req.options) {
                        Some(option_id) => responder.respond(RequestPermissionResponse::new(
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                                option_id,
                            )),
                        )),
                        None => responder
                            .respond_with_error(agent_client_protocol::Error::method_not_found()),
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async move |notif: SessionNotification, _cx| {
                    fold_update(
                        &notif_config,
                        &notif_fold,
                        &notif_sink,
                        &notif_label,
                        notif.update,
                    );
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            );

        let cwd = opts.cwd.clone();
        let mcp_servers = mcp_servers.clone();

        let result: Result<(), agent_client_protocol::Error> = builder
            .connect_with(ByteStreams::new(stdin, stdout), move |conn| async move {
                // --- initialize handshake + session/new ---
                match proto::handshake(&conn, cwd, mcp_servers).await {
                    Ok(hs) => {
                        // Capture the advertised options as the session's
                        // initial option state.
                        if let Some(options) = hs.config_options {
                            *driver_config.lock().unwrap() = options;
                        }
                        // No rendered readiness line here: the scripting
                        // layer emits the structured `SessionReady` event
                        // once the handle exists, and the renderer
                        // formats the verbose-only line from it. The
                        // ready handshake below still gates `start_session`.
                        let _ = ready_tx.send(Ok(hs.session_id.clone()));
                        run_command_loop(
                            &conn,
                            &mut cmd_rx,
                            &fold,
                            &driver_config,
                            &driver_sink,
                            &driver_label,
                            hs.session_id,
                        )
                        .await
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(SessionError::Handshake(e)));
                        Ok(())
                    }
                }
            })
            .await;

        if let Err(e) = result {
            tracing::debug!(%e, "agent connection ended");
        }
        kill_and_reap(child_guard, driver_groups.as_deref()).await;
        // Drain the stderr pump so -vv passthrough is complete before the
        // session is reported closed.
        let _ = stderr_task.await;
        // Tear the result channel down with the session: stop accepting,
        // unlink the socket, and — if nothing was ever submitted through
        // it — note the (designed) degradation once.
        if let Some(channel) = result_channel {
            let had_results = channel.any_accepted();
            channel.close().await;
            if !had_results {
                sink.emit(
                    &teardown_label,
                    SessionEvent::Lifecycle {
                        message: format!(
                            "{teardown_label}: session ended without typed results \
                             (agent never submitted through the result tool)"
                        ),
                    },
                );
            }
        }
        let _ = done_tx.send(true);
    });

    match ready_rx.await {
        Ok(Ok(session_id)) => Ok(SessionHandle {
            label,
            session_id: session_id.to_string(),
            pid,
            cmd_tx,
            done_rx,
            turn_lock: Arc::new(tokio::sync::Mutex::new(())),
            config_options,
        }),
        Ok(Err(e)) => {
            let _ = driver.await;
            Err(e)
        }
        Err(_) => {
            let _ = driver.await;
            Err(SessionError::DriverDied(
                "driver exited before ready".into(),
            ))
        }
    }
}

/// The ACP stdio transport: [`AgentTransport`] over the process/proto/
/// driver interior.
pub struct Transport {
    /// Where every spawned agent's group-leader pid is registered for
    /// the composition root's second-signal sweep. `None`: untracked
    /// (tests, embedders that install no monitor).
    groups: Option<Arc<ProcessGroups>>,
}

impl Transport {
    /// An untracked transport: sessions spawn and tear down normally,
    /// but nothing records their pids for an outer-signal sweep.
    pub fn new() -> Self {
        Self { groups: None }
    }

    /// A transport registering every spawned agent's group-leader pid
    /// in `groups` for the run's lifetime — the registry handle the
    /// composition-root signal monitor sweeps on the force escape.
    pub fn with_registry(groups: Arc<ProcessGroups>) -> Self {
        Self {
            groups: Some(groups),
        }
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTransport for Transport {
    fn start_session<'a>(
        &'a self,
        spec: &'a AgentSpec,
        opts: SessionOptions,
        sink: Arc<dyn EventSink>,
    ) -> Pin<Box<dyn Future<Output = Result<SessionHandle, SessionError>> + 'a>> {
        Box::pin(start_session_inner(
            spec,
            opts,
            sink,
            self.groups.clone(),
        ))
    }
}

/// Serve the command channel until `close`: prompt turns and set-config
/// runs spawn on the connection (responses deliver through oneshots),
/// `cancel` notifies the agent, `close` ends the loop.
async fn run_command_loop(
    conn: &ConnectionTo<agent_client_protocol::Agent>,
    cmd_rx: &mut mpsc::UnboundedReceiver<SessionCmd>,
    fold: &Arc<Mutex<TurnFold>>,
    driver_config: &Arc<Mutex<Vec<SessionConfigOption>>>,
    driver_sink: &Arc<dyn EventSink>,
    driver_label: &str,
    session_id: agent_client_protocol::schema::v1::SessionId,
) -> Result<(), agent_client_protocol::Error> {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            SessionCmd::Prompt {
                text,
                timeout,
                resp,
            } => {
                let conn2 = conn.clone();
                let fold = fold.clone();
                let sink = driver_sink.clone();
                let session_id = session_id.clone();
                let label = driver_label.to_string();
                let spawned = conn.spawn(async move {
                    let outcome =
                        run_turn(&conn2, &fold, &sink, &label, &session_id, text, timeout).await;
                    let _ = resp.send(outcome);
                    sink.emit(&label, SessionEvent::TurnEnd);
                    Ok(())
                });
                // If queueing failed the closure (and `resp` with it)
                // was dropped: the awaiting prompt observes a closed
                // channel and raises `TurnError::Closed`.
                if let Err(e) = spawned {
                    tracing::warn!(%e, "failed to queue prompt task");
                }
            }
            SessionCmd::SetConfig { id, value, resp } => {
                let conn2 = conn.clone();
                let config = driver_config.clone();
                let sink = driver_sink.clone();
                let session_id = session_id.clone();
                let label = driver_label.to_string();
                let spawned = conn.spawn(async move {
                    let result =
                        run_set_config(&conn2, &config, &sink, &label, &session_id, id, value)
                            .await;
                    let _ = resp.send(result);
                    Ok(())
                });
                // If queueing failed the closure (and `resp` with
                // it) was dropped: the awaiting setConfig call
                // observes a closed channel and errors.
                if let Err(e) = spawned {
                    tracing::warn!(%e, "failed to queue set-config task");
                }
            }
            SessionCmd::Cancel => {
                if let Err(e) = conn.send_notification(CancelNotification::new(session_id.clone()))
                {
                    tracing::warn!(%e, "failed to send session/cancel");
                }
            }
            SessionCmd::Close => break,
        }
    }

    Ok(())
}

/// Drive one prompt turn: send `session/prompt`, race the deadline, fold
/// streaming updates, and deliver the outcome.
#[allow(clippy::too_many_arguments)]
async fn run_turn(
    conn: &ConnectionTo<agent_client_protocol::Agent>,
    fold: &Arc<Mutex<TurnFold>>,
    sink: &Arc<dyn EventSink>,
    label: &str,
    session_id: &agent_client_protocol::schema::v1::SessionId,
    text: String,
    timeout: Option<Duration>,
) -> Result<TurnOutcome, TurnError> {
    // Fresh slot per turn; submissions landing before this point are late.
    fold.lock().unwrap().begin_turn();

    // Exactly one prompt line per turn, at send time, attributed to this
    // session (render-logging "Prompt turns render a prompt line"). The
    // sink gates it on `--quiet` like every rendered line.
    sink.emit(label, SessionEvent::Prompt { text: text.clone() });

    let req = PromptRequest::new(
        session_id.clone(),
        vec![ContentBlock::Text(TextContent::new(text))],
    );

    let (tx, mut rx) = oneshot::channel();
    conn.send_request(req)
        .on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })
        .map_err(|e| TurnError::Agent(e.to_string()))?;

    let response: Result<PromptResponse, TurnError> = match timeout {
        None => match rx.await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(TurnError::Agent(e.to_string())),
            Err(_) => Err(TurnError::Closed("agent connection ended".into())),
        },
        Some(limit) => match tokio::time::timeout(limit, &mut rx).await {
            Ok(Ok(Ok(resp))) => Ok(resp),
            Ok(Ok(Err(e))) => Err(TurnError::Agent(e.to_string())),
            Ok(Err(_)) => Err(TurnError::Closed("agent connection ended".into())),
            Err(_elapsed) => {
                // Timed out: cancel remotely, then give the agent a bounded
                // grace period to land its (cancelled) response before
                // raising the timeout error regardless.
                let _ = conn.send_notification(CancelNotification::new(session_id.clone()));
                let _ = tokio::time::timeout(CANCEL_GRACE, rx).await;
                Err(TurnError::Timeout)
            }
        },
    };

    let resp = match response {
        Ok(resp) => resp,
        Err(e) => {
            // Cancelled / timed out / failed: the turn's text and any
            // submission it had gathered are discarded (settle drains both
            // so nothing leaks into the next turn on this session).
            let _ = fold.lock().unwrap().settle_turn(true);
            return Err(e);
        }
    };
    let stop_reason = stop_reason_string(&resp.stop_reason);
    // A cancelled turn's partial text is as unreliable as its discarded
    // submission; any other completion settles normally.
    let (text, result) = fold.lock().unwrap().settle_turn(stop_reason == "cancelled");
    Ok(TurnOutcome {
        text,
        stop_reason,
        usage: resp
            .usage
            .as_ref()
            .map(UsageCounts::from_usage)
            .unwrap_or_default(),
        result,
    })
}

/// Fold one streaming update into the turn accumulator + sink.
fn fold_update(
    config: &Arc<Mutex<Vec<SessionConfigOption>>>,
    fold: &Arc<Mutex<TurnFold>>,
    sink: &Arc<dyn EventSink>,
    label: &str,
    update: SessionUpdate,
) {
    match update {
        SessionUpdate::AgentMessageChunk(ContentChunk {
            content: ContentBlock::Text(t),
            ..
        }) => {
            let mut fold = fold.lock().unwrap();
            let message_break = fold.push_text(&t.text);
            drop(fold);
            sink.emit(
                label,
                SessionEvent::TextDelta {
                    delta: t.text,
                    message_break,
                },
            );
        }
        SessionUpdate::AgentMessageChunk(_) => {}
        SessionUpdate::ToolCall(call) => {
            let line = {
                let mut fold = fold.lock().unwrap();
                fold.break_message();
                let inputs = PeekInputs {
                    kind: Some(&call.kind),
                    locations: Some(&call.locations),
                    raw_input: call.raw_input.as_ref(),
                };
                fold.tools.announce(
                    &call.tool_call_id.0,
                    &call.title,
                    &status_string(&call.status),
                    &inputs,
                    Instant::now(),
                )
            };
            if let Some(line) = line {
                sink.emit(label, SessionEvent::ToolLine(line));
            }
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let line = {
                let mut fold = fold.lock().unwrap();
                fold.break_message();
                fold.tools
                    .update(&update.tool_call_id.0, &update.fields, Instant::now())
            };
            if let Some(line) = line {
                sink.emit(label, SessionEvent::ToolLine(line));
            }
        }
        SessionUpdate::Plan(plan) => {
            let entries: Vec<PlanEntry> = plan
                .entries
                .iter()
                .map(|e| PlanEntry {
                    status: plan_status(&e.status),
                    content: e.content.clone(),
                })
                .collect();
            sink.emit(label, SessionEvent::Plan { entries });
        }
        SessionUpdate::UsageUpdate(u) => {
            sink.emit(
                label,
                SessionEvent::Usage {
                    used: u.used,
                    size: u.size,
                },
            );
        }
        SessionUpdate::ConfigOptionUpdate(update) => {
            // The payload carries the full option set: replace the state
            // wholesale and note what changed (no reply exists — it's a
            // notification).
            let changed = apply_config_options(config, update.config_options);
            if !changed.is_empty() {
                sink.emit(
                    label,
                    SessionEvent::Lifecycle {
                        message: format!("{label}: config changed: {}", format_changed(&changed)),
                    },
                );
            }
        }
        // User message echo, thoughts, and unstable updates are not rendered in v1.
        _ => {}
    }
}

/// Send one `session/set_config_option` and fold the response into the
/// session's option state. The caller holds the session's turn lock, so
/// this runs strictly between turns.
async fn run_set_config(
    conn: &ConnectionTo<agent_client_protocol::Agent>,
    config: &Arc<Mutex<Vec<SessionConfigOption>>>,
    sink: &Arc<dyn EventSink>,
    label: &str,
    session_id: &agent_client_protocol::schema::v1::SessionId,
    id: String,
    value: SessionConfigOptionValue,
) -> Result<(), String> {
    let req = SetSessionConfigOptionRequest::new(
        session_id.clone(),
        SessionConfigId::new(id.clone()),
        value.clone(),
    );
    match proto::request(conn, req).await {
        Ok(resp) => {
            let changed = apply_config_options(config, resp.config_options);
            // One lifecycle line per successful set, naming each changed
            // option id and its new value (falls back to the requested
            // pair when the agent reports no diff).
            let summary = if changed.is_empty() {
                format!("{id}={}", option_value_display(&value))
            } else {
                format_changed(&changed)
            };
            sink.emit(
                label,
                SessionEvent::Lifecycle {
                    message: format!("{label}: config changed: {summary}"),
                },
            );
            Ok(())
        }
        Err(e) => Err(format!("setConfig(\"{id}\") failed: {e}")),
    }
}

/// Replace the session's option state wholesale and return the
/// `(id, new value)` pairs that differ from the previous state. An update
/// arriving with no prior option state reports every advertised option
/// as changed.
fn apply_config_options(
    config: &Arc<Mutex<Vec<SessionConfigOption>>>,
    new_options: Vec<SessionConfigOption>,
) -> Vec<(String, String)> {
    let mut options = config.lock().unwrap();
    let previous: std::collections::HashMap<String, String> = options
        .iter()
        .filter_map(|o| Some((o.id.0.to_string(), current_value_string(o)?)))
        .collect();
    let changed = new_options
        .iter()
        .filter_map(|o| {
            let id = o.id.0.to_string();
            let value = current_value_string(o)?;
            (previous.get(&id).map(String::as_str) != Some(value.as_str())).then_some((id, value))
        })
        .collect();
    *options = new_options;
    changed
}

/// Display string for an option's current value (select value id or
/// boolean); `None` for unknown option kinds.
fn current_value_string(opt: &SessionConfigOption) -> Option<String> {
    match &opt.kind {
        SessionConfigKind::Select(s) => Some(s.current_value.0.to_string()),
        SessionConfigKind::Boolean(b) => Some(b.current_value.to_string()),
        _ => None,
    }
}

/// Display string for a `set_config_option` value.
fn option_value_display(value: &SessionConfigOptionValue) -> String {
    if let Some(id) = value.as_value_id() {
        id.0.to_string()
    } else if let Some(b) = value.as_bool() {
        b.to_string()
    } else {
        String::new()
    }
}

/// `id=value, id=value` summary of a changed-options list.
fn format_changed(changed: &[(String, String)]) -> String {
    changed
        .iter()
        .map(|(id, value)| format!("{id}={value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Map a protocol plan status to the protocol-agnostic event status.
fn plan_status(status: &agent_client_protocol::schema::v1::PlanEntryStatus) -> PlanStatus {
    use agent_client_protocol::schema::v1::PlanEntryStatus::{Completed, InProgress, Pending};
    match status {
        Pending => PlanStatus::Pending,
        InProgress => PlanStatus::InProgress,
        Completed => PlanStatus::Completed,
        _ => PlanStatus::Other,
    }
}

fn stop_reason_string(reason: &StopReason) -> String {
    match reason {
        StopReason::EndTurn => "end_turn".into(),
        StopReason::MaxTokens => "max_tokens".into(),
        StopReason::MaxTurnRequests => "max_turn_requests".into(),
        StopReason::Refusal => "refusal".into(),
        StopReason::Cancelled => "cancelled".into(),
        _ => "end_turn".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_config_options_diff() {
        use agent_client_protocol::schema::v1::{
            SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions,
        };

        let config = Arc::new(Mutex::new(Vec::<SessionConfigOption>::new()));
        let choices = vec![
            SessionConfigSelectOption::new("opus", "Opus"),
            SessionConfigSelectOption::new("haiku", "Haiku"),
        ];
        let options = vec![
            SessionConfigOption::select(
                "model",
                "Model",
                "opus",
                SessionConfigSelectOptions::Ungrouped(choices.clone()),
            ),
            SessionConfigOption::boolean("fast", "Fast mode", false),
        ];

        // No prior state: every advertised option counts as changed.
        let changed = apply_config_options(&config, options.clone());
        assert_eq!(
            changed,
            vec![
                ("model".to_string(), "opus".to_string()),
                ("fast".to_string(), "false".to_string()),
            ]
        );

        // Identical state: nothing changed.
        let changed = apply_config_options(&config, options.clone());
        assert!(changed.is_empty(), "{changed:?}");

        // New value (and a previously unseen id): both reported.
        let mut updated = options;
        updated[0] = SessionConfigOption::select(
            "model",
            "Model",
            "haiku",
            SessionConfigSelectOptions::Ungrouped(choices),
        );
        updated.push(SessionConfigOption::boolean("effort", "Effort", true));
        let changed = apply_config_options(&config, updated);
        assert_eq!(
            changed,
            vec![
                ("model".to_string(), "haiku".to_string()),
                ("effort".to_string(), "true".to_string()),
            ]
        );
    }
}
