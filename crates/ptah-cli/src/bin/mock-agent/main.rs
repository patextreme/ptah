//! mock-agent: a scriptable ACP agent used by ptah's integration tests.
//!
//! Speaks the agent side of ACP v1 over stdio. Behavior is driven by
//! environment variables so tests can pick a scenario per session:
//!
//! - `MOCK_CHUNKS`      — `|`-separated chunks streamed per prompt
//!   (default: echo the prompt text as one chunk)
//! - `MOCK_LEAD_CHUNKS` — `|`-separated chunks streamed at turn start,
//!   after client-request probes and before tool activity: the fixture
//!   for narrate → act → answer turns (final `r.text` keeps only the
//!   last message)
//! - `MOCK_DELAY_MS`    — delay between streamed chunks (default 0)
//! - `MOCK_USAGE`       — comma list `used,size,in,out,cache_read,cache_write`:
//!   emits a `usage_update` during the turn and carries
//!   token usage on the prompt response
//! - `MOCK_TOOL`        — emit a `tool_call` update (pending → completed)
//! - `MOCK_TOOL_KIND`   — `ToolKind` of the emitted tool calls (default
//!   `other`; e.g. `execute`, `read`, `edit`)
//! - `MOCK_TOOL_TITLE`  — title of the emitted tool calls (defaults:
//!   `mock_tool` for `MOCK_TOOL`, `Search files "foo"` for
//!   `MOCK_TOOL_FLOW`; e.g. `bash` for pi-acp-style dedup fixtures)
//! - `MOCK_TOOL_LOCATIONS` — comma-separated `path[:line]` absolute
//!   locations attached to the emitted tool calls (peek paths)
//! - `MOCK_TOOL_RAW_INPUT` — JSON value attached as the emitted tool
//!   calls' `rawInput` (peek commands and fallback JSON)
//! - `MOCK_TOOL_FLOW`   — comma-separated status sequence replayed for one
//!   tool call: a titled `tool_call` for the first entry,
//!   `tool_call_update`s for the rest (e.g.
//!   `pending,in_progress,in_progress,completed`). Prefix an entry with
//!   `!` to force it out as a bare update (an id that was never announced
//!   — ptah's raw-id fallback). `MOCK_DELAY_MS` spaces the entries.
//! - `MOCK_PLAN`        — emit a plan update
//! - `MOCK_PERMISSION`  — send `session/request_permission`; `once`/`1`
//!   offers only `AllowOnce`, `always` offers `AllowOnce` + `AllowAlways`
//!   (asserts the client selects `allow_always`), `reject` offers only
//!   reject options (asserts the client's unsupported-method error)
//! - `MOCK_REQUEST`    — `|`-separated method names sent verbatim as
//!   agent-to-client requests during each prompt (asserts the client
//!   answers each with the unsupported-method `-32601` error); use for
//!   `fs/read_text_file`, `fs/write_text_file`, `terminal/*`,
//!   `elicitation/create`, or any unknown method
//! - `MOCK_HANG`        — never respond to prompts unless cancelled
//! - `MOCK_STDERR`      — write this text to stderr once per prompt
//! - `MOCK_STOP_REASON` — override the stop reason (default `end_turn`)
//! - `MOCK_ENV_DUMP`    — name of an env var: each prompt's reply text is
//!   that variable's value (for env-inheritance tests)
//! - `MOCK_ECHO_CWD`    — each prompt replies with the session's `cwd`
//!   (for default-cwd tests)
//! - `MOCK_ECHO_MCP`    — each prompt replies with the JSON of the
//!   `mcpServers` config received at `session/new`
//! - `MOCK_MCP_LIST`    — each prompt replies with a JSON listing of the
//!   tools offered by the injected `ptah` server
//! - `MOCK_NO_MCP`      — ignore the session's `mcpServers` entirely
//!   (spec-legal degradation of suggested servers)
//! - `MOCK_MCP_UNSPAWNABLE` — server name whose spawn is sabotaged with a
//!   nonexistent command (simulates an agent sandboxed away from the
//!   binary — the degrade path, turn must still complete)
//! - `MOCK_SUBMIT`      — `|`-separated JSON values; each prompt calls the
//!   `result_submit` tool of the `ptah` server with each value in order
//!   (last one wins in ptah)
//! - `MOCK_SUBMIT_MATCH` — JSON array of `{"match": "…", "value": …}`
//!   rules; each prompt scans them in order and submits the value of
//!   the first rule whose `match` substring occurs in the prompt text.
//!   A rule may carry `requiresConfig` (object of config id → required
//!   current value): the rule only matches when the mock's live option
//!   state equals every required value — order-sensitive config-state
//!   gating for session-config propagation tests (state re-seeds at
//!   every `session/new`). No matching rule → no submission (typed
//!   result stays nil), which is how tests script judge retries and
//!   per-iteration verdicts from one stateless mock. Takes precedence
//!   over `MOCK_SUBMIT`.
//! - `MOCK_SUBMIT_ONCE` — with `MOCK_SUBMIT`, submit only on the first
//!   prompt (fresh-slot-per-turn tests)
//! - `MOCK_SUBMIT_BAD`  — number of invalid submissions (value from
//!   `MOCK_SUBMIT_BAD_VALUE`, default `{}`) to send first, asserting each
//!   returns a tool error naming violations, before the `MOCK_SUBMIT`
//!   values (in-turn retry proof); with `MOCK_SUBMIT_BAD_NEEDLE` each
//!   violation text must also contain that substring
//! - `MOCK_CONFIG_OPTIONS` — JSON array of `SessionConfigOption` echoed as
//!   the `configOptions` of the `session/new` response and seeded as the
//!   mock's live option state (asserts ptah advertises the
//!   `session.configOptions` client capability with its `boolean`
//!   sub-capability in `initialize`)
//! - `MOCK_CONFIG_REJECT` — fail `session/set_config_option` for the named
//!   config id: a generic JSON-RPC error for plain `id`, method-not-found
//!   (-32601) for `id=notfound`; other ids succeed and mutate the mock's
//!   in-memory option state
//! - `MOCK_CONFIG_REJECT_DELAY_MS` — with `MOCK_CONFIG_REJECT`, hold the
//!   rejection response for this many milliseconds first (keeps the agent
//!   process observably alive mid-handshake; constructor-teardown tests)
//! - `MOCK_EOF_LINGER` — park forever after the ACP connection ends
//!   instead of exiting, so only an explicit teardown reap (never stdin
//!   EOF) ends the process (run-end sweep tests)
//! - `MOCK_CONFIG_UPDATE` — JSON array of options: after the first
//!   prompt completes, push a `config_option_update` (`session/update`
//!   payload) carrying the new full option set and apply it to the
//!   in-memory state
//! - `MOCK_CONFIG_ECHO`   — config id: each turn's reply echoes that
//!   option's current in-memory value (end-to-end proof that prompts run
//!   under a changed config)
//! - `MOCK_CONFIG_DEPENDENT` — any non-empty value: a
//!   `session/set_config_option` for `model` re-derives `effort` to its
//!   seeded default in the response state, modeling agents with
//!   dependent options (opencode-style model → effort reset); pins the
//!   setConfig sequencing contract: driving option first, dependent
//!   option last
//!
//! The mock is also an MCP client (rmcp): stdio servers suggested in
//! `session/new { mcpServers }` are spawned, handshaked, and torn down
//! with the session. Children are shut down gracefully when the ACP
//! connection ends.
//!
//! `session/cancel` marks the in-flight turn cancelled: the awaiting prompt
//! responds with `stopReason: "cancelled"`.

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AgentRequest, CancelNotification, ConfigOptionUpdate, ContentBlock,
    ContentChunk, ExtRequest, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionKind, Plan, PlanEntry, PlanEntryPriority,
    PlanEntryStatus, PromptRequest, PromptResponse, RequestPermissionRequest,
    SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigOptionValue,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse, StopReason, TextContent, ToolCall, ToolCallLocation,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, Usage, UsageUpdate,
};
use agent_client_protocol::{Agent, ConnectionTo, Stdio};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::TokioChildProcess;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Notify;

/// Bound on one MCP server handshake so a wedged child cannot hang a test.
const MCP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy)]
struct MockUsage {
    used: u64,
    size: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

impl MockUsage {
    fn from_env() -> Option<Self> {
        let raw = std::env::var("MOCK_USAGE").ok()?;
        let parts: Vec<u64> = raw
            .split(',')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect();
        let get = |i: usize| parts.get(i).copied().unwrap_or(0);
        Some(Self {
            used: get(0),
            size: get(1),
            input: get(2),
            output: get(3),
            cache_read: get(4),
            cache_write: get(5),
        })
    }
}

/// `RunningService` is neither `Clone` nor shareable behind one owner, so
/// each client lives behind an `Arc<Mutex<…>>` (calls are sequential in
/// every scripted scenario anyway).
type SharedMcpClient = Arc<tokio::sync::Mutex<rmcp::service::RunningService<rmcp::RoleClient, ()>>>;

/// MCP servers spawned from the session's `mcpServers` (stdio only).
#[derive(Default)]
struct McpState {
    clients: std::sync::Mutex<Vec<(String, SharedMcpClient)>>,
    /// Raw `mcpServers` config received at session/new (MOCK_ECHO_MCP).
    servers_json: std::sync::Mutex<Option<serde_json::Value>>,
    /// Prompt counter (MOCK_SUBMIT_ONCE).
    prompts: AtomicU64,
    /// Set when a `ptah` server was offered (diagnostics).
    saw_ptah: AtomicBool,
}

impl McpState {
    fn client(&self, name: &str) -> Option<SharedMcpClient> {
        self.clients
            .lock()
            .unwrap()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| Arc::clone(c))
    }

    async fn shutdown(&self) {
        let clients = std::mem::take(&mut *self.clients.lock().unwrap());
        for (_, client) in clients {
            let _ = client.lock().await.close().await;
        }
    }
}

/// Live per-session config-option state (`MOCK_CONFIG_OPTIONS` seed,
/// mutated by `session/set_config_option` and `MOCK_CONFIG_UPDATE`, read
/// by `MOCK_CONFIG_ECHO`; `MOCK_CONFIG_DEPENDENT` resets dependent
/// options back into `defaults`).
#[derive(Default)]
struct ConfigState {
    options: std::sync::Mutex<Vec<SessionConfigOption>>,
    /// Seed snapshot (`MOCK_CONFIG_OPTIONS`): the values
    /// `MOCK_CONFIG_DEPENDENT` re-derives dependent options to.
    defaults: std::sync::Mutex<Vec<SessionConfigOption>>,
    /// Prompt counter (`MOCK_CONFIG_UPDATE` pushes once, on the first).
    prompts: AtomicU64,
}

impl ConfigState {
    /// Current display value of one option by id (select value id or
    /// boolean); `unknown-config:<id>` when absent.
    fn echo_value(&self, id: &str) -> String {
        let options = self.options.lock().unwrap();
        for opt in options.iter() {
            if opt.id.0.as_ref() == id {
                return match &opt.kind {
                    SessionConfigKind::Select(s) => s.current_value.0.to_string(),
                    SessionConfigKind::Boolean(b) => b.current_value.to_string(),
                    _ => String::new(),
                };
            }
        }
        format!("unknown-config:{id}")
    }
}

#[derive(Default)]
struct TurnState {
    cancelled: AtomicBool,
    cancel_notify: Notify,
    /// `cwd` of the (single) session, for MOCK_ECHO_CWD.
    session_cwd: std::sync::Mutex<Option<String>>,
}

impl TurnState {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // notify_one stores a permit when nobody is waiting yet, so a cancel
        // that lands before (or while) a prompt registers is never lost.
        self.cancel_notify.notify_one();
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

fn env_ms(name: &str) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Parse one `MOCK_TOOL_FLOW` status entry.
fn tool_status_from_str(s: &str) -> ToolCallStatus {
    match s {
        "pending" => ToolCallStatus::Pending,
        "in_progress" => ToolCallStatus::InProgress,
        "completed" => ToolCallStatus::Completed,
        "failed" => ToolCallStatus::Failed,
        other => panic!("mock-agent: unknown MOCK_TOOL_FLOW status {other:?}"),
    }
}

fn stop_reason_from_env() -> StopReason {
    match std::env::var("MOCK_STOP_REASON").as_deref() {
        Ok("max_tokens") => StopReason::MaxTokens,
        Ok("max_turn_requests") => StopReason::MaxTurnRequests,
        Ok("refusal") => StopReason::Refusal,
        Ok("cancelled") => StopReason::Cancelled,
        _ => StopReason::EndTurn,
    }
}

/// `MOCK_TOOL_KIND` as a `ToolKind` (default `other`; unknown values
/// panic so test typos fail loudly).
fn tool_kind_from_env() -> ToolKind {
    match std::env::var("MOCK_TOOL_KIND").as_deref() {
        Ok("read") => ToolKind::Read,
        Ok("edit") => ToolKind::Edit,
        Ok("delete") => ToolKind::Delete,
        Ok("move") => ToolKind::Move,
        Ok("search") => ToolKind::Search,
        Ok("execute") => ToolKind::Execute,
        Ok("think") => ToolKind::Think,
        Ok("fetch") => ToolKind::Fetch,
        Ok("switch_mode") => ToolKind::SwitchMode,
        Ok("other") | Err(_) => ToolKind::Other,
        Ok(other) => panic!("mock-agent: unknown MOCK_TOOL_KIND {other:?}"),
    }
}

/// `MOCK_TOOL_LOCATIONS` as locations: comma-separated `path[:line]`
/// entries (`/abs/path.rs:12`); a `:line` suffix is optional.
fn tool_locations_from_env() -> Vec<ToolCallLocation> {
    let raw = match std::env::var("MOCK_TOOL_LOCATIONS") {
        Ok(r) if !r.is_empty() => r,
        _ => return Vec::new(),
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|entry| match entry.rsplit_once(':') {
            Some((path, line)) if !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) => {
                ToolCallLocation::new(path).line(
                    line.parse::<u32>()
                        .expect("MOCK_TOOL_LOCATIONS line number"),
                )
            }
            _ => ToolCallLocation::new(entry),
        })
        .collect()
}

/// `MOCK_TOOL_RAW_INPUT` as a JSON value.
fn tool_raw_input_from_env() -> Option<serde_json::Value> {
    std::env::var("MOCK_TOOL_RAW_INPUT")
        .ok()
        .filter(|r| !r.is_empty())
        .map(|raw| serde_json::from_str(&raw).expect("MOCK_TOOL_RAW_INPUT must be JSON"))
}

/// A `ToolCall` announcement for `MOCK_TOOL`/`MOCK_TOOL_FLOW`, carrying
/// the `MOCK_TOOL_KIND` / `MOCK_TOOL_LOCATIONS` / `MOCK_TOOL_RAW_INPUT`
/// knobs so ptah's peek synthesis is drivable end to end.
fn mock_tool_call(id: &str, default_title: &str, status: ToolCallStatus) -> ToolCall {
    let title = std::env::var("MOCK_TOOL_TITLE")
        .ok()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| default_title.to_string());
    let mut call = ToolCall::new(id.to_string(), title)
        .kind(tool_kind_from_env())
        .status(status);
    let locations = tool_locations_from_env();
    if !locations.is_empty() {
        call = call.locations(locations);
    }
    if let Some(raw) = tool_raw_input_from_env() {
        call = call.raw_input(raw);
    }
    call
}

fn prompt_text(req: &PromptRequest) -> String {
    req.prompt
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<String>()
}

/// Spawn and handshake the stdio servers suggested in `session/new`.
/// Failures degrade: the server is skipped (with a stderr note), never
/// fatal — agents are free to ignore suggested servers, and so is the mock.
async fn start_mcp_servers(
    mcp: &Arc<McpState>,
    servers: &[agent_client_protocol::schema::v1::McpServer],
) {
    use agent_client_protocol::schema::v1::McpServer;
    *mcp.servers_json.lock().unwrap() = Some(serde_json::to_value(servers).unwrap_or_default());
    if env_flag("MOCK_NO_MCP") {
        return;
    }
    // Replace any servers from a previous session on this connection.
    mcp.shutdown().await;
    for server in servers {
        let McpServer::Stdio(stdio) = server else {
            eprintln!("mock-agent: skipping non-stdio MCP server");
            continue;
        };
        if stdio.name == "ptah" {
            mcp.saw_ptah.store(true, Ordering::SeqCst);
        }
        // Test hook: sabotage the spawn of one named server with a
        // nonexistent command, simulating an agent sandboxed away from
        // the binary (spec degradation path — turn must still complete).
        let unspawnable = std::env::var("MOCK_MCP_UNSPAWNABLE").ok();
        let command_path = if unspawnable.as_deref() == Some(stdio.name.as_str()) {
            std::path::PathBuf::from("/nonexistent/mock-agent-sabotage")
        } else {
            stdio.command.clone()
        };
        let mut command = tokio::process::Command::new(command_path);
        command.args(&stdio.args);
        for var in &stdio.env {
            command.env(&var.name, &var.value);
        }
        let transport = match TokioChildProcess::new(command) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("mock-agent: cannot spawn MCP server {}: {e}", stdio.name);
                continue;
            }
        };
        match tokio::time::timeout(MCP_HANDSHAKE_TIMEOUT, ().serve(transport)).await {
            Ok(Ok(client)) => mcp.clients.lock().unwrap().push((
                stdio.name.clone(),
                Arc::new(tokio::sync::Mutex::new(client)),
            )),
            Ok(Err(e)) => {
                eprintln!("mock-agent: MCP handshake with {} failed: {e}", stdio.name);
            }
            Err(_) => eprintln!("mock-agent: MCP handshake with {} timed out", stdio.name),
        }
    }
}

/// Text of a tool result (concatenated text blocks).
fn result_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<String>()
}

/// Call `result_submit` on the `ptah` server with `value`.
async fn submit_result(client: &SharedMcpClient, value: &serde_json::Value) -> CallToolResult {
    let mut arguments = serde_json::Map::new();
    arguments.insert("value".to_string(), value.clone());
    let client = client.lock().await;
    client
        .call_tool(CallToolRequestParams::new("result_submit").with_arguments(arguments))
        .await
        .expect("result_submit round-trip")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let turn = Arc::new(TurnState::default());
    let mcp = Arc::new(McpState::default());
    let config = Arc::new(ConfigState::default());

    let builder = Agent
        .builder()
        .on_receive_request(
            async |req: InitializeRequest,
                   responder: agent_client_protocol::Responder<InitializeResponse>,
                   _cx| {
                // Capability gating: assert the client advertises
                // `session.configOptions` (with its `boolean` sub-capability).
                let cap = req
                    .client_capabilities
                    .session
                    .as_ref()
                    .and_then(|s| s.config_options.as_ref());
                assert!(
                    cap.is_some_and(|c| c.boolean.is_some()),
                    "client must advertise session.configOptions (+boolean) in initialize"
                );
                responder.respond(InitializeResponse::new(req.protocol_version))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let turn = turn.clone();
                let mcp = mcp.clone();
                let config = config.clone();
                async move |req: NewSessionRequest,
                            responder: agent_client_protocol::Responder<NewSessionResponse>,
                            _cx| {
                    let n = session_counter.fetch_add(1, Ordering::SeqCst) + 1;
                    *turn.session_cwd.lock().unwrap() = Some(req.cwd.display().to_string());
                    start_mcp_servers(&mcp, &req.mcp_servers).await;
                    let _ = AgentCapabilities::new();
                    let mut response = NewSessionResponse::new(format!("mock-session-{n}"));
                    if let Some(raw) = std::env::var("MOCK_CONFIG_OPTIONS")
                        .ok()
                        .filter(|r| !r.is_empty())
                    {
                        let options: Vec<SessionConfigOption> = serde_json::from_str(&raw)
                            .expect("MOCK_CONFIG_OPTIONS must be a JSON array of config options");
                        config.options.lock().unwrap().clone_from(&options);
                        config.defaults.lock().unwrap().clone_from(&options);
                        response = response.config_options(options);
                    }
                    responder.respond(response)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let config = config.clone();
                async move |req: SetSessionConfigOptionRequest,
                            responder: agent_client_protocol::Responder<
                    SetSessionConfigOptionResponse,
                >,
                            _cx| {
                    // MOCK_CONFIG_REJECT: plain `<id>` → generic error;
                    // `<id>=notfound` → method-not-found (-32601).
                    // MOCK_CONFIG_REJECT_DELAY_MS holds the rejection back
                    // (tests observe the live process before teardown).
                    if let Some(reject) = std::env::var("MOCK_CONFIG_REJECT")
                        .ok()
                        .filter(|r| !r.is_empty())
                    {
                        let (rid, notfound) = match reject.strip_suffix("=notfound") {
                            Some(id) => (id.to_string(), true),
                            None => (reject.clone(), false),
                        };
                        if req.config_id.0.as_ref() == rid {
                            if let Some(ms) = std::env::var("MOCK_CONFIG_REJECT_DELAY_MS")
                                .ok()
                                .filter(|v| !v.is_empty())
                            {
                                tokio::time::sleep(Duration::from_millis(ms.parse().unwrap_or(0)))
                                    .await;
                            }
                            let error = if notfound {
                                agent_client_protocol::Error::method_not_found()
                            } else {
                                agent_client_protocol::Error::new(
                                    -32000,
                                    format!("mock-agent rejects config id {rid}"),
                                )
                            };
                            return responder.respond_with_error(error);
                        }
                    }
                    // Other ids: mutate the in-memory state, echo the full set.
                    let mut options = config.options.lock().unwrap();
                    for opt in options.iter_mut() {
                        if opt.id.0.as_ref() == req.config_id.0.as_ref() {
                            match (&mut opt.kind, &req.value) {
                                (
                                    SessionConfigKind::Select(s),
                                    SessionConfigOptionValue::ValueId { value },
                                ) => s.current_value = value.clone(),
                                (
                                    SessionConfigKind::Boolean(b),
                                    SessionConfigOptionValue::Boolean { value },
                                ) => b.current_value = *value,
                                _ => {}
                            }
                        }
                    }
                    // MOCK_CONFIG_DEPENDENT: model agents whose option sets
                    // have internal dependencies — a `model` set re-derives
                    // `effort` to its seeded default, so only a later
                    // `effort` set (driving-option-first sequencing) sticks.
                    if std::env::var("MOCK_CONFIG_DEPENDENT").is_ok_and(|v| !v.is_empty())
                        && req.config_id.0.as_ref() == "model"
                    {
                        let default = config
                            .defaults
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|o| o.id.0.as_ref() == "effort")
                            .map(|o| o.kind.clone());
                        if let Some(kind) = default {
                            for opt in options.iter_mut() {
                                if opt.id.0.as_ref() == "effort" {
                                    opt.kind = kind.clone();
                                }
                            }
                        }
                    }
                    responder.respond(SetSessionConfigOptionResponse::new(options.clone()))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let turn = turn.clone();
                let mcp = mcp.clone();
                async move |req: PromptRequest,
                            responder: agent_client_protocol::Responder<PromptResponse>,
                            cx: ConnectionTo<agent_client_protocol::Client>| {
                    let turn = turn.clone();
                    let mcp = mcp.clone();
                    let config = config.clone();
                    let conn = cx.clone();
                    cx.spawn(async move {
                        if let Err(e) = run_prompt(req, responder, conn, turn, mcp, config).await {
                            eprintln!("mock-agent: prompt failed: {e}");
                        }
                        Ok(())
                    })?;
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let turn = turn.clone();
                async move |_notif: CancelNotification, _cx| {
                    turn.cancel();
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        );

    builder.connect_to(Stdio::new()).await?;
    // MOCK_EOF_LINGER: park after the connection ends instead of exiting,
    // so a missed teardown leaves a live process tests can observe (ptah
    // kills the whole process group explicitly; EOF alone never reaps
    // this mode).
    if env_flag("MOCK_EOF_LINGER") {
        std::future::pending::<()>().await;
    }
    // The ACP connection ended: tear the spawned MCP servers down with the
    // session before the runtime goes away.
    mcp.shutdown().await;
    Ok(())
}

async fn run_prompt(
    req: PromptRequest,
    responder: agent_client_protocol::Responder<PromptResponse>,
    cx: ConnectionTo<agent_client_protocol::Client>,
    turn: Arc<TurnState>,
    mcp: Arc<McpState>,
    config: Arc<ConfigState>,
) -> Result<(), agent_client_protocol::Error> {
    let session_id = req.session_id.clone();
    let text = prompt_text(&req);
    let delay = Duration::from_millis(env_ms("MOCK_DELAY_MS"));

    // Each turn starts with fresh cancellation state: a `session/cancel`
    // targets in-flight work, not future prompts. (A cancel that races the
    // prompt request is still honored via the notify permit.)
    turn.cancelled.store(false, Ordering::SeqCst);

    if let Some(msg) = std::env::var("MOCK_STDERR").ok().filter(|m| !m.is_empty()) {
        eprintln!("{msg}");
    }

    // Optional permission round-trip: the client answers with an offered
    // allow option (headless allow-all) or an unsupported-method error
    // when the offer has no allow option. Which shape is offered — and
    // which selection is therefore expected — is picked by MOCK_PERMISSION.
    if let Some(mode) = std::env::var("MOCK_PERMISSION")
        .ok()
        .filter(|m| !m.is_empty())
    {
        let options = match mode.as_str() {
            "always" => vec![
                PermissionOption::new("allow_once", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new(
                    "allow_always",
                    "Always allow",
                    PermissionOptionKind::AllowAlways,
                ),
            ],
            "reject" => vec![
                PermissionOption::new("reject_once", "Reject", PermissionOptionKind::RejectOnce),
                PermissionOption::new(
                    "reject_always",
                    "Always reject",
                    PermissionOptionKind::RejectAlways,
                ),
            ],
            _ => vec![PermissionOption::new(
                "allow_once",
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        };
        let expected_id = match mode.as_str() {
            "always" => "allow_always",
            "reject" => "",
            _ => "allow_once",
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tool_call = ToolCallUpdate::new(
            "tool-perm",
            ToolCallUpdateFields::new().status(ToolCallStatus::Pending),
        );
        cx.send_request(RequestPermissionRequest::new(
            session_id.clone(),
            tool_call,
            options,
        ))
        .on_receiving_result(async move |result| {
            let _ = tx.send(result);
            Ok(())
        })?;
        match rx.await {
            Ok(Ok(resp)) => {
                assert_eq!(
                    resp.outcome,
                    agent_client_protocol::schema::v1::RequestPermissionOutcome::Selected(
                        SelectedPermissionOutcome::new(expected_id)
                    ),
                    "client selected the wrong permission option for mode {mode:?}"
                );
            }
            Ok(Err(e)) => {
                let msg = format!("{e:?}");
                if msg.contains("incoming_transport_closed") {
                    // Client vanished mid-request: proceed without asserting.
                } else if mode == "reject" {
                    assert!(
                        msg.contains("ethod not found") || msg.contains("-32601"),
                        "reject-only offer should get -32601, got: {msg}"
                    );
                } else {
                    panic!("client should have allowed mode {mode:?}, got: {msg}");
                }
            }
            Err(_) => {} // transport gone: proceed
        }
    }

    // Arbitrary agent-to-client requests: each method named in MOCK_REQUEST
    // is sent verbatim on the wire (via the untagged extension variant),
    // and the client must answer with the unsupported-method (-32601)
    // error — never a result, never silence. This pins the headless
    // catch-all for fs/read_text_file, fs/write_text_file, terminal/*,
    // elicitation/create, and unknown methods alike.
    if let Some(list) = std::env::var("MOCK_REQUEST").ok().filter(|l| !l.is_empty()) {
        for method in list.split('|').map(str::trim).filter(|m| !m.is_empty()) {
            let params: std::sync::Arc<serde_json::value::RawValue> = std::sync::Arc::from(
                serde_json::value::RawValue::from_string("{}".to_string())
                    .expect("empty params object"),
            );
            let (tx, rx) = tokio::sync::oneshot::channel();
            cx.send_request(AgentRequest::ExtMethodRequest(ExtRequest::new(
                method, params,
            )))
            .on_receiving_result(async move |result| {
                let _ = tx.send(result);
                Ok(())
            })?;
            match rx.await {
                Ok(Ok(value)) => {
                    panic!("client should answer {method:?} with method-not-found, got: {value:?}")
                }
                Ok(Err(e)) => {
                    let msg = format!("{e:?}");
                    if msg.contains("incoming_transport_closed") {
                        // Client vanished mid-request: proceed without asserting.
                    } else {
                        assert!(
                            msg.contains("ethod not found") || msg.contains("-32601"),
                            "{method} should get -32601, got: {msg}"
                        );
                    }
                }
                Err(_) => {} // transport gone: proceed
            }
        }
    }

    // Lead message: streamed at turn start, before any tool activity,
    // so tests can script the narrate → act → answer turn shape
    // (MOCK_LEAD_CHUNKS). Same delay/cancel discipline as the final
    // message below.
    if let Some(lead) = std::env::var("MOCK_LEAD_CHUNKS")
        .ok()
        .filter(|l| !l.is_empty())
    {
        for chunk in lead.split('|') {
            if turn.cancelled.load(Ordering::SeqCst) {
                return respond_cancelled(responder);
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if turn.cancelled.load(Ordering::SeqCst) {
                return respond_cancelled(responder);
            }
            cx.send_notification(SessionNotification::new(
                session_id.clone(),
                SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new(chunk),
                ))),
            ))?;
        }
    }

    if env_flag("MOCK_TOOL") {
        cx.send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::ToolCall(mock_tool_call(
                "tool-1",
                "mock_tool",
                ToolCallStatus::Pending,
            )),
        ))?;
        tokio::time::sleep(delay).await;
        if turn.cancelled.load(Ordering::SeqCst) {
            return respond_cancelled(responder);
        }
        cx.send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool-1",
                ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            )),
        ))?;
    }

    // Arbitrary status replay for one tool call: a titled announcement
    // for the first entry, updates for the rest — the fixture for ptah's
    // tool-line transition/dedup policy. `!`-prefixed entries go out as
    // bare updates even in first position (unannounced ids).
    if let Some(seq) = std::env::var("MOCK_TOOL_FLOW")
        .ok()
        .filter(|s| !s.is_empty())
    {
        const FLOW_ID: &str = "tool-flow-1";
        const FLOW_TITLE: &str = "Search files \"foo\"";
        for (i, raw) in seq
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .enumerate()
        {
            let (bare, status) = match raw.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, raw),
            };
            let status = tool_status_from_str(status);
            let update = if i == 0 && !bare {
                SessionUpdate::ToolCall(mock_tool_call(FLOW_ID, FLOW_TITLE, status))
            } else {
                SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    FLOW_ID,
                    ToolCallUpdateFields::new().status(status),
                ))
            };
            cx.send_notification(SessionNotification::new(session_id.clone(), update))?;
            tokio::time::sleep(delay).await;
            if turn.cancelled.load(Ordering::SeqCst) {
                return respond_cancelled(responder);
            }
        }
    }

    if env_flag("MOCK_PLAN") {
        cx.send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::Plan(Plan::new(vec![PlanEntry::new(
                "mock step",
                PlanEntryPriority::Medium,
                PlanEntryStatus::InProgress,
            )])),
        ))?;
    }

    if let Some(u) = MockUsage::from_env() {
        cx.send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::UsageUpdate(UsageUpdate::new(u.used, u.size)),
        ))?;
    }

    // Typed-result submissions: exercised when the session offered a
    // `ptah` server (i.e. the script declared a result contract).
    if let Some(client) = mcp.client("ptah") {
        let n = mcp.prompts.fetch_add(1, Ordering::SeqCst) + 1;
        let should_submit = !env_flag("MOCK_SUBMIT_ONCE") || n == 1;
        // Prompt-content-keyed rules (MOCK_SUBMIT_MATCH): the first rule
        // whose `match` substring occurs in the prompt text drives this
        // turn's submission; no match submits nothing.
        let match_rules: Option<Vec<serde_json::Value>> = std::env::var("MOCK_SUBMIT_MATCH")
            .ok()
            .filter(|r| !r.is_empty())
            .map(|raw| {
                serde_json::from_str(&raw)
                    .expect("MOCK_SUBMIT_MATCH must be a JSON array of {match,value} rules")
            });
        let attempted = should_submit
            && (match_rules.is_some()
                || std::env::var("MOCK_SUBMIT").is_ok()
                || std::env::var("MOCK_SUBMIT_BAD").is_ok());
        let mut submitted_ok = false;
        if should_submit {
            if let Some(rules) = &match_rules {
                for rule in rules {
                    let needle = rule.get("match").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.contains(needle) {
                        continue;
                    }
                    // `requiresConfig`: the rule fires only when the live
                    // option state equals every required value (same value
                    // formatting the MOCK_CONFIG_ECHO reply uses). A gated
                    // rule that fails its gate is skipped, not consumed —
                    // later rules still get their chance.
                    if let Some(required) = rule.get("requiresConfig").and_then(|v| v.as_object()) {
                        let satisfied = required.iter().all(|(id, want)| {
                            config.echo_value(id) == want.as_str().unwrap_or_default()
                        });
                        if !satisfied {
                            continue;
                        }
                    }
                    let value = rule
                        .get("value")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let result = submit_result(&client, &value).await;
                    assert!(
                        result.is_error != Some(true),
                        "matched submission should be accepted, got: {:?}",
                        result_text(&result)
                    );
                    submitted_ok = true;
                    break;
                }
            } else {
                if let Ok(bad_count) = std::env::var("MOCK_SUBMIT_BAD") {
                    let bad_count: usize = bad_count.parse().unwrap_or(0);
                    let bad_value = std::env::var("MOCK_SUBMIT_BAD_VALUE")
                        .ok()
                        .and_then(|v| serde_json::from_str(&v).ok())
                        .unwrap_or(serde_json::json!({}));
                    let needle = std::env::var("MOCK_SUBMIT_BAD_NEEDLE").ok();
                    for _ in 0..bad_count {
                        let result = submit_result(&client, &bad_value).await;
                        assert!(
                            result.is_error == Some(true),
                            "invalid submission should be a tool error, got: {:?}",
                            result_text(&result)
                        );
                        let violations = result_text(&result);
                        assert!(!violations.is_empty(), "violation text must not be empty");
                        if let Some(needle) = &needle {
                            assert!(
                                violations.contains(needle.as_str()),
                                "violation text must name the violation ({needle:?}), got: {violations:?}"
                            );
                        }
                    }
                }
                if let Ok(values) = std::env::var("MOCK_SUBMIT") {
                    for raw in values.split('|') {
                        let value: serde_json::Value =
                            serde_json::from_str(raw).expect("MOCK_SUBMIT entries must be JSON");
                        let result = submit_result(&client, &value).await;
                        assert!(
                            result.is_error != Some(true),
                            "valid submission should be accepted, got: {:?}",
                            result_text(&result)
                        );
                        submitted_ok = true;
                    }
                }
            }
        }
        // Surface the submit as a tool-call update so renderers show it.
        if attempted {
            cx.send_notification(SessionNotification::new(
                session_id.clone(),
                SessionUpdate::ToolCall(
                    ToolCall::new("submit-1", "mcp__ptah__result_submit").status(if submitted_ok {
                        ToolCallStatus::Completed
                    } else {
                        ToolCallStatus::Failed
                    }),
                ),
            ))?;
        }
    }

    // Agent-pushed config update: once the first prompt completes, replace
    // the in-memory option state and notify the client with the new full
    // set (mid-session push; the notification follows the prompt response
    // on the wire, so it is folded before any later turn starts).
    let config_update = std::env::var("MOCK_CONFIG_UPDATE")
        .ok()
        .filter(|r| !r.is_empty())
        .and_then(|raw| {
            serde_json::from_str::<Vec<SessionConfigOption>>(&raw).map_or_else(
                |e| panic!("MOCK_CONFIG_UPDATE must be a JSON array of config options: {e}"),
                Some,
            )
        });

    let chunks: Vec<String> = match std::env::var("MOCK_ENV_DUMP") {
        Ok(var) => vec![std::env::var(&var).unwrap_or_default()],
        Err(_) if env_flag("MOCK_ECHO_CWD") => {
            vec![turn.session_cwd.lock().unwrap().clone().unwrap_or_default()]
        }
        Err(_) if env_flag("MOCK_ECHO_MCP") => {
            let json = mcp.servers_json.lock().unwrap().clone().unwrap_or_default();
            vec![serde_json::to_string(&json).unwrap_or_default()]
        }
        Err(_) if env_flag("MOCK_MCP_LIST") => match mcp.client("ptah") {
            Some(client) => {
                let tools = client
                    .lock()
                    .await
                    .list_tools(None)
                    .await
                    .expect("tools/list on ptah server");
                let listing: Vec<serde_json::Value> = tools
                    .tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "inputSchema": serde_json::Value::Object(t.input_schema.as_ref().clone()),
                        })
                    })
                    .collect();
                vec![serde_json::to_string(&listing).unwrap_or_default()]
            }
            None => vec!["no-ptah-server".to_string()],
        },
        Err(_) if env_flag("MOCK_CONFIG_ECHO") => {
            let id = std::env::var("MOCK_CONFIG_ECHO").unwrap_or_default();
            vec![config.echo_value(&id)]
        }
        Err(_) => match std::env::var("MOCK_CHUNKS") {
            Ok(spec) => spec
                .split('|')
                .map(std::string::ToString::to_string)
                .collect(),
            Err(_) => vec![text],
        },
    };

    if env_flag("MOCK_HANG") {
        // Never complete the turn on our own: wait for a cancel.
        if !turn.cancelled.load(Ordering::SeqCst) {
            turn.cancel_notify.notified().await;
        }
        return respond_cancelled(responder);
    }

    for chunk in chunks {
        if turn.cancelled.load(Ordering::SeqCst) {
            return respond_cancelled(responder);
        }
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if turn.cancelled.load(Ordering::SeqCst) {
            return respond_cancelled(responder);
        }
        cx.send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(chunk),
            ))),
        ))?;
    }

    let mut response = PromptResponse::new(stop_reason_from_env());
    if let Some(u) = MockUsage::from_env() {
        response.usage = Some(Usage::new(
            u.input + u.output + u.cache_read + u.cache_write,
            u.input,
            u.output,
        ));
    }
    responder.respond(response)?;

    // After the first prompt only: apply and push the config update.
    if let Some(options) = config_update {
        let n = config.prompts.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 {
            config.options.lock().unwrap().clone_from(&options);
            cx.send_notification(SessionNotification::new(
                session_id.clone(),
                SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
            ))?;
        }
    }
    Ok(())
}

fn respond_cancelled(
    responder: agent_client_protocol::Responder<PromptResponse>,
) -> Result<(), agent_client_protocol::Error> {
    responder.respond(PromptResponse::new(StopReason::Cancelled))
}
