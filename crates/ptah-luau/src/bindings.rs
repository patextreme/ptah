//! The `ptah.*` namespace bindings and the object constructors handed
//! to scripts: task/session/agent-factory tables (plain tables with
//! closure methods; userdata metatables would be built lazily by mlua,
//! which re-reads the `coroutine` global we hide — so userdata is avoided
//! entirely).

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use mlua::LuaSerdeExt;
use mlua::{Function, Lua, MultiValue, Table, Value};

use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption,
};

use ptah_core::config::AgentSpec;
use ptah_core::error::ExitSignal;
use ptah_core::events::{AskAction, SessionEvent};
use ptah_core::ports::{AskError, AskOutcome, AskRequest, ExecError, InteractionMode};
use ptah_core::session::{SessionHandle, SessionOptions};
use ptah_core::task::{self, TaskState};

use super::state::{EXEC_LABEL, ExecEntry, runtime_state};

fn new_task_obj(lua: &Lua, state: Rc<TaskState>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let s = state;
    t.set(
        "await",
        lua.create_async_function(move |_lua, _self: Table| {
            let s = s.clone();
            async move { s.await_result().await }
        })?,
    )?;
    Ok(t)
}

fn new_session_obj(lua: &Lua, handle: SessionHandle) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    let prompt_handle = handle.clone();
    t.set(
        "prompt",
        lua.create_async_function(
            move |lua, (_self, text, opts): (Table, String, Option<Table>)| {
                let handle = prompt_handle.clone();
                async move {
                    let timeout = match &opts {
                        Some(t) => t
                            .get::<Option<u64>>("timeoutMs")?
                            .map(Duration::from_millis),
                        None => None,
                    };
                    // Turn serialization lives in the session handle.
                    let outcome = handle
                        .prompt(text, timeout)
                        .await
                        .map_err(|e| mlua::Error::runtime(e.to_string()))?;

                    let result = lua.create_table()?;
                    result.set("text", outcome.text.clone())?;
                    result.set("stopReason", outcome.stop_reason)?;
                    let usage = lua.create_table()?;
                    usage.set("input", outcome.usage.input)?;
                    usage.set("cacheRead", outcome.usage.cache_read)?;
                    usage.set("cacheWrite", outcome.usage.cache_write)?;
                    usage.set("output", outcome.usage.output)?;
                    result.set("usage", usage)?;
                    // The turn's last accepted typed submission as a Luau
                    // value, or nil when there was none. JSON null also
                    // arrives as nil (mlua's default would produce its
                    // null userdata sentinel instead).
                    let submitted = outcome.result.as_ref().map_or(Value::Nil, |json| {
                        lua.to_value_with(
                            json,
                            mlua::serde::ser::Options::new()
                                .serialize_none_to_null(false)
                                .serialize_unit_to_null(false),
                        )
                        .unwrap_or(Value::Nil)
                    });
                    result.set("result", submitted)?;

                    let meta = lua.create_table()?;
                    meta.set(
                        "__tostring",
                        lua.create_function(|_lua, t: Table| {
                            let text: String = t.get("text")?;
                            Ok(text)
                        })?,
                    )?;
                    result.set_metatable(Some(meta))?;
                    Ok(result)
                }
            },
        )?,
    )?;

    let cancel_handle = handle.clone();
    t.set(
        "cancel",
        lua.create_function(move |_lua, _self: Table| {
            cancel_handle.cancel();
            Ok(())
        })?,
    )?;

    let label = handle.label.clone();
    t.set(
        "label",
        lua.create_function(move |_lua, _self: Table| Ok(label.clone()))?,
    )?;

    let config_handle = handle.clone();
    t.set(
        "configOptions",
        lua.create_function(move |lua, _self: Table| {
            let options = config_handle.config_options();
            config_options_table(lua, &options)
        })?,
    )?;

    let set_config_handle = handle.clone();
    t.set(
        "setConfig",
        lua.create_async_function(move |_lua, (_self, id, value): (Table, String, Value)| {
            let handle = set_config_handle.clone();
            async move {
                // Value typing happens before anything is sent: a Luau
                // string is a select value id, a boolean is a boolean
                // option value, anything else is a script error.
                let wire_value = match value {
                    Value::String(s) => {
                        let id = s.to_str()?.to_string();
                        SessionConfigOptionValue::value_id(id)
                    }
                    Value::Boolean(b) => SessionConfigOptionValue::boolean(b),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "setConfig value must be a string (select value id) or boolean, \
                                 got {}",
                            other.type_name()
                        )));
                    }
                };
                handle
                    .set_config(id, wire_value)
                    .await
                    .map_err(mlua::Error::runtime)
            }
        })?,
    )?;

    let close_handle = handle;
    t.set(
        "close",
        lua.create_async_function(move |lua, _self: Table| {
            let handle = close_handle.clone();
            async move {
                let state = runtime_state(&lua)?;
                handle.close();
                handle.join().await;
                // Remove by pid (session identity), not label: two
                // factories for one agent name can have live sessions
                // sharing a label, and this registry is the run-end
                // teardown list — a label match would unregister the
                // survivor and strand its subprocess.
                state.sessions.borrow_mut().retain(|s| s.pid != handle.pid);
                Ok(())
            }
        })?,
    )?;

    Ok(t)
}

/// Convert the session's config-option state to a Luau array of option
/// tables: `{ id, name, type ("select"|"boolean"), currentValue, category?,
/// options? }` — select entries carry an `options` list of
/// `{ id, name, description? }` choices (grouped selects are flattened);
/// `category` is set only when the agent provides one (UX hint only).
fn config_options_table(lua: &Lua, options: &[SessionConfigOption]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, opt) in options.iter().enumerate() {
        let e = lua.create_table()?;
        e.set("id", opt.id.0.to_string())?;
        e.set("name", opt.name.clone())?;
        match &opt.kind {
            SessionConfigKind::Select(s) => {
                e.set("type", "select")?;
                e.set("currentValue", s.current_value.0.to_string())?;
                let choices = lua.create_table()?;
                for (n, choice) in flatten_select_options(&s.options).into_iter().enumerate() {
                    let c = lua.create_table()?;
                    c.set("id", choice.value.0.to_string())?;
                    c.set("name", choice.name.clone())?;
                    if let Some(d) = &choice.description {
                        c.set("description", d.clone())?;
                    }
                    choices.raw_set(n + 1, c)?;
                }
                e.set("options", choices)?;
            }
            SessionConfigKind::Boolean(b) => {
                e.set("type", "boolean")?;
                e.set("currentValue", b.current_value)?;
            }
            _ => continue, // unknown option kinds are skipped
        }
        if let Some(category) = &opt.category
            && let serde_json::Value::String(name) =
                serde_json::to_value(category).unwrap_or(serde_json::Value::Null)
        {
            e.set("category", name)?;
        }
        t.raw_set(i + 1, e)?;
    }
    Ok(t)
}

/// Flatten a select option's choices (grouped selects contribute every
/// group's options, in order).
fn flatten_select_options(
    options: &agent_client_protocol::schema::v1::SessionConfigSelectOptions,
) -> Vec<&SessionConfigSelectOption> {
    use agent_client_protocol::schema::v1::SessionConfigSelectOptions;
    match options {
        SessionConfigSelectOptions::Ungrouped(list) => list.iter().collect(),
        SessionConfigSelectOptions::Grouped(groups) => {
            groups.iter().flat_map(|g| g.options.iter()).collect()
        }
        _ => Vec::new(),
    }
}

fn new_agent_factory(lua: &Lua, name: String, spec: AgentSpec) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let name_rc = Rc::new(name);
    let spec_rc = Rc::new(spec);
    let counter = Rc::new(Cell::new(0u64));
    t.set(
        "session",
        lua.create_async_function(move |lua, (_self, opts): (Table, Option<Table>)| {
            let name = name_rc.clone();
            let spec = spec_rc.clone();
            let counter = counter.clone();
            async move {
                let state = runtime_state(&lua)?;
                let opts = match opts {
                    Some(t) => t,
                    None => lua.create_table()?,
                };

                let id: Option<String> = opts.get("id")?;
                // `exec` is the sink's reserved pseudo-label for
                // script-level exec lifecycle events; a user session id
                // of `exec` would make script activity render (and
                // future TUI-track) as a session. Reserved forever.
                if id.as_deref() == Some(crate::state::EXEC_LABEL) {
                    return Err(mlua::Error::runtime(
                        "session id `exec` is reserved for exec lifecycle \
                         attribution — choose another id",
                    ));
                }
                let n = counter.get() + 1;
                counter.set(n);
                let id = id.unwrap_or_else(|| format!("s{n}"));
                let label = format!("{name}/{id}");

                let cwd: Option<String> = opts.get("cwd")?;
                let cwd = match cwd {
                    Some(dir) => {
                        let p = Path::new(&dir);
                        if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            state.invocation_dir.join(p)
                        }
                    }
                    None => state.invocation_dir.clone(),
                };

                let mut mcp_servers = Vec::new();
                let raw: Option<Value> = opts.get("mcpServers")?;
                if let Some(raw) = raw {
                    let json: serde_json::Value = lua.from_value(raw)?;
                    mcp_servers = serde_json::from_value(json).map_err(|e| {
                        mlua::Error::runtime(format!("invalid mcpServers entry: {e}"))
                    })?;
                }

                // The `config` session option is removed (it was a table
                // applied with unspecified `pairs()` order, which cannot
                // coexist with agents that re-derive dependent options).
                // The key itself is the removed API: reject it pre-spawn —
                // populated or empty — instead of silently ignoring old
                // scripts; the author migrates to sequential setConfig
                // calls in dependency order (driving options first).
                if opts.get::<Option<Table>>("config")?.is_some() {
                    return Err(mlua::Error::runtime(
                        "config session option was removed: a config table cannot express \
                         application order, which matters for agents with dependent options \
                         (e.g. opencode resets `effort` when `model` is set). Apply config \
                         with session:setConfig(...) after session creation — set driving \
                         options (like `model`) first",
                    ));
                }

                // Typed result contract: eager compilation so schema errors
                // fail at the author's line, before any subprocess spawns.
                let result =
                    match opts.get::<Option<Value>>("resultSchema")? {
                        Some(raw) => {
                            let json: serde_json::Value = lua.from_value(raw).map_err(|e| {
                                mlua::Error::runtime(format!("invalid result schema: {e}"))
                            })?;
                            Some(ptah_core::contract::ResultContract::compile(json).map_err(
                                |e| mlua::Error::runtime(format!("invalid result schema: {e}")),
                            )?)
                        }
                        None => None,
                    };

                state.sink.emit(
                    &label,
                    SessionEvent::Lifecycle {
                        message: format!("{label}: spawning agent"),
                    },
                );
                let handle = state
                    .transport
                    .start_session(
                        &spec,
                        SessionOptions {
                            cwd,
                            mcp_servers,
                            label: label.clone(),
                            result,
                        },
                        state.sink.clone(),
                    )
                    .await
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                state.sessions.borrow_mut().push(handle.clone());

                new_session_obj(&lua, handle)
            }
        })?,
    )?;
    Ok(t)
}
fn interp_lookup(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

pub(super) fn bind_ptah(lua: &Lua) -> mlua::Result<()> {
    let ptah = lua.create_table()?;

    // ptah.agent(name_or_spec)
    let agent = lua.create_async_function(|lua, spec: Value| async move {
        let state = runtime_state(&lua)?;
        let resolved = match &spec {
            Value::String(name) => {
                let name = name.to_str()?.to_string();
                state
                    .registry
                    .resolve_with(&name, &interp_lookup)
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?
            }
            Value::Table(t) => {
                let args: Option<Vec<String>> = t.get("args")?;
                let env: Option<std::collections::BTreeMap<String, String>> = t.get("env")?;
                AgentSpec {
                    command: t.get("command")?,
                    args: args.unwrap_or_default(),
                    env: env.unwrap_or_default(),
                }
                .interpolate(&interp_lookup)
            }
            other => {
                // mlua's `BadArgument::cause` is an `Arc<Error>`; without the
                // `send` feature `Error` is !Sync, which is fine here.
                #[allow(clippy::arc_with_non_send_sync)]
                let cause = std::sync::Arc::new(mlua::Error::runtime(format!(
                    "expected string or table, got {}",
                    other.type_name()
                )));
                return Err(mlua::Error::BadArgument {
                    to: Some("ptah.agent".into()),
                    pos: 1,
                    name: Some("name_or_spec".into()),
                    cause,
                });
            }
        };
        let name = match &spec {
            Value::String(name) => name.to_str()?.to_string(),
            _ => resolved.command.clone(),
        };
        new_agent_factory(&lua, name, resolved)
    })?;
    ptah.set("agent", agent)?;

    // ptah.spawn(fn)
    let spawn = lua.create_function(|lua, f: Function| {
        let state = runtime_state(lua)?;
        let task_state = task::spawn(lua, &state.tasks, f)?;
        Ok(new_task_obj(lua, task_state))
    })?;
    ptah.set("spawn", spawn)?;

    // ptah.join({task, ...}) -> outcome entries
    let join = lua.create_async_function(|lua, tasks: Table| async move {
        let outcomes = lua.create_table()?;
        let len = tasks.raw_len();
        for i in 1..=len {
            let entry_task: Table = tasks.get(i)?;
            let await_fn: Function = entry_task
                .get("await")
                .map_err(|_| mlua::Error::runtime("join expects task objects"))?;
            let res: mlua::Result<MultiValue> = await_fn.call_async((entry_task.clone(),)).await;
            let entry = outcome_entry(&lua, res)?;
            outcomes.raw_set(i, entry)?;
        }
        Ok(outcomes)
    })?;
    ptah.set("join", join)?;

    // ptah.parallel(items, fn, {concurrency}) -> outcome entries in item order
    let parallel = lua.create_async_function(
        |lua, (items, f, opts): (Table, Function, Option<Table>)| async move {
            let state = runtime_state(&lua)?;
            let concurrency: usize = match &opts {
                Some(t) => {
                    let c: Option<usize> = t.get("concurrency")?;
                    c.unwrap_or(usize::MAX)
                }
                None => usize::MAX,
            };
            let concurrency = concurrency.max(1);

            let mut item_values: Vec<Value> = Vec::new();
            for i in 1..=items.raw_len() {
                item_values.push(items.get(i)?);
            }

            let outcomes = lua.create_table()?;
            let mut idx = 0;
            for chunk in item_values.chunks(concurrency) {
                // Launch the chunk concurrently.
                let mut states = Vec::new();
                for item in chunk {
                    let state_rc = Rc::new(TaskState::default());
                    state.tasks.register(state_rc.clone());
                    let fut = f.call_async::<MultiValue>(item.clone());
                    let s = state_rc.clone();
                    tokio::task::spawn_local(async move {
                        let result = match fut.await {
                            Ok(v) => task::TaskResult::Value(v),
                            Err(e) => task::TaskResult::Error(e),
                        };
                        s.complete(result);
                    });
                    states.push(state_rc);
                }
                // Await the chunk; errors become outcome entries, not failures.
                for s in states {
                    let res = s.await_result().await;
                    idx += 1;
                    let entry = outcome_entry(&lua, res)?;
                    outcomes.raw_set(idx, entry)?;
                }
            }
            Ok(outcomes)
        },
    )?;
    ptah.set("parallel", parallel)?;

    // ptah.sleep(ms)
    let sleep = lua.create_async_function(|_lua, ms: u64| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(())
    })?;
    ptah.set("sleep", sleep)?;

    // ptah.ask({ prompt, details? }) — suspend the calling coroutine while
    // a human answers (only the caller parks: other tasks, in-flight
    // turns, and agent sessions keep progressing). Response-or-abort is
    // data; the four failure conditions (prohibited, no provider,
    // provider failure, end of input) raise distinct messages. The ask
    // lock serializes concurrent asks FIFO — taken before the request
    // event is emitted, held until the resolution is emitted, so
    // providers (including test fakes) see exactly one ask at a time.
    let ask = lua.create_async_function(|lua, opts: Option<Value>| async move {
        let state = runtime_state(&lua)?;
        let usage = |msg: String| mlua::Error::runtime(format!("ptah.ask: {msg}"));

        let opts = match opts {
            Some(Value::Table(t)) => t,
            Some(Value::Nil) | None => {
                return Err(usage(
                    "expected a table argument { prompt = \"…\", details = \"…\"? }".into(),
                ))
            }
            Some(other) => {
                return Err(usage(format!(
                    "expected a table argument {{ prompt = \"…\", details = \"…\"? }}, got {}",
                    other.type_name()
                )))
            }
        };
        let prompt = match opts.get::<Option<Value>>("prompt")? {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            Some(other) => {
                return Err(usage(format!(
                    "`prompt` must be a string, got {}",
                    other.type_name()
                )))
            }
            None => {
                return Err(usage(
                    "missing required `prompt` field (string)".into(),
                ))
            }
        };
        let details = match opts.get::<Option<Value>>("details")? {
            Some(Value::String(s)) => Some(s.to_str()?.to_string()),
            Some(Value::Nil) | None => None,
            Some(other) => {
                return Err(usage(format!(
                    "`details` must be a string, got {}",
                    other.type_name()
                )))
            }
        };

        let provider = match &state.interaction {
            InteractionMode::Provider(p) => Arc::clone(p),
            InteractionMode::Prohibited => {
                return Err(mlua::Error::runtime(
                    "ptah.ask: interaction is prohibited — the run resolved to the `none` \
                     provider (--ask=none, PTAH_ASK=none, or [ask] provider = \"none\")",
                ))
            }
            InteractionMode::Unresolved => {
                return Err(mlua::Error::runtime(
                    "ptah.ask: no ask provider configured — pass --ask, set PTAH_ASK, or \
                     configure [ask] (auto-detection needs a terminal on stdin and stdout)",
                ))
            }
        };

        // Per-run attribution: `ask {n} {script_basename}`.
        let n = state.ask_counter.get() + 1;
        state.ask_counter.set(n);
        let script_name = state
            .script_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = format!("ask {n} {script_name}");

        // Serialize: hold the ask lock from before the request event
        // until the resolution is emitted (or the ask is dropped).
        let _guard = state.ask_lock.lock().await;
        state.sink.emit(
            &label,
            SessionEvent::AskRequested {
                prompt: prompt.clone(),
                details: details.clone(),
            },
        );
        let outcome = provider
            .ask(AskRequest {
                prompt,
                details,
                attribution: label.clone(),
            })
            .await;
        match outcome {
            Ok(AskOutcome::Respond { text }) => {
                state.sink.emit(
                    &label,
                    SessionEvent::AskResolved {
                        action: AskAction::Respond,
                        text: Some(text.clone()),
                    },
                );
                let result = lua.create_table()?;
                result.set("action", "respond")?;
                result.set("text", text)?;
                Ok(result)
            }
            Ok(AskOutcome::Abort) => {
                state.sink.emit(
                    &label,
                    SessionEvent::AskResolved {
                        action: AskAction::Abort,
                        text: None,
                    },
                );
                let result = lua.create_table()?;
                result.set("action", "abort")?;
                Ok(result)
            }
            // No resolution event on failures: the ask did not resolve,
            // it raised (only teardown or a human answer resolves).
            Err(AskError::InputClosed) => Err(mlua::Error::runtime(
                "ptah.ask: end of input — the ask provider's input closed with no answer \
                 (stdin EOF on a non-terminal)",
            )),
            Err(AskError::Failed(msg)) => Err(mlua::Error::runtime(format!(
                "ptah.ask: ask provider failed: {msg}"
            ))),
        }
    })?;
    ptah.set("ask", ask)?;

    // ptah.log(msg)
    let log = lua.create_function(|lua, msg: String| {
        let state = runtime_state(lua)?;
        state.sink.script_log(&msg);
        Ok(())
    })?;
    ptah.set("log", log)?;

    // ptah.exec(cmd, opts?) — run a shell command through the injected
    // ProcessRunner capability (blocking the calling coroutine; spawned
    // tasks and other turns keep progressing). Nonzero exit is data;
    // only could-not-run and timeout raise.
    let exec =
        lua.create_async_function(|lua, (cmd, opts): (String, Option<Value>)| async move {
            let state = runtime_state(&lua)?;
            let timeout_ms = match opts {
                None | Some(Value::Nil) => None,
                Some(Value::Table(t)) => t.get::<Option<u64>>("timeoutMs")?,
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "ptah.exec: opts must be a table ({{ timeoutMs = 100 }}), got {}",
                        other.type_name()
                    )));
                }
            };
            let runner = state.process_runner.clone().ok_or_else(|| {
                mlua::Error::runtime("ptah.exec: no process runner injected into this runtime")
            })?;

            state.sink.emit(
                EXEC_LABEL,
                SessionEvent::ExecStart {
                    command: cmd.clone(),
                },
            );
            // In-flight registration: teardown signals `cancel` so this
            // future (not just the Lua runtime) drops now — and dropping
            // the port future is the runner's kill-the-group contract.
            // A teardown that already fired shows up as `killed` here
            // (checked before the future is ever polled, so the child
            // never spawns).
            let entry = Rc::new(ExecEntry::default());
            state.execs.borrow_mut().push(entry.clone());
            let started = std::time::Instant::now();
            let fut = runner.run(&cmd, timeout_ms);
            let outcome = if entry.killed.get() {
                None
            } else {
                tokio::select! {
                    _ = entry.cancel.notified() => None,
                    out = fut => Some(out),
                }
            };
            state.execs.borrow_mut().retain(|e| !Rc::ptr_eq(e, &entry));
            let Some(outcome) = outcome else {
                return Err(mlua::Error::runtime(format!(
                    "exec `{cmd}` cancelled: the run is ending"
                )));
            };
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

            let outcome = match outcome {
                Ok(o) => o,
                Err(e @ ExecError::Spawn(_)) => {
                    state.sink.emit(
                        EXEC_LABEL,
                        SessionEvent::ExecEnd {
                            command: cmd.clone(),
                            exit_code: None,
                            timed_out: false,
                            duration_ms,
                        },
                    );
                    return Err(mlua::Error::runtime(format!("ptah.exec: {e}")));
                }
            };
            if outcome.timed_out {
                state.sink.emit(
                    EXEC_LABEL,
                    SessionEvent::ExecEnd {
                        command: cmd.clone(),
                        exit_code: None,
                        timed_out: true,
                        duration_ms,
                    },
                );
                let budget = timeout_ms
                    .map(|ms| format!(" after {ms}ms"))
                    .unwrap_or_default();
                return Err(mlua::Error::runtime(format!(
                    "ptah.exec `{cmd}` timed out{budget}"
                )));
            }
            state.sink.emit(
                EXEC_LABEL,
                SessionEvent::ExecEnd {
                    command: cmd.clone(),
                    exit_code: outcome.exit_code,
                    timed_out: false,
                    duration_ms,
                },
            );

            let result = lua.create_table()?;
            result.set("exitCode", outcome.exit_code.unwrap_or(-1))?;
            result.set("stdout", outcome.stdout)?;
            result.set("stderr", outcome.stderr)?;
            Ok(result)
        })?;
    ptah.set("exec", exec)?;

    // ptah.json — pure JSON encode/decode (no port, no I/O): turns
    // captured command output into script data and back.
    let json = lua.create_table()?;
    json.set(
        "parse",
        lua.create_function(|lua, s: String| {
            let v: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| mlua::Error::runtime(format!("ptah.json.parse: {e}")))?;
            Ok(lua.to_value_with(
                &v,
                mlua::serde::ser::Options::new()
                    .serialize_none_to_null(false)
                    .serialize_unit_to_null(false),
            ))
        })?,
    )?;
    json.set(
        "stringify",
        lua.create_function(|lua, (value, opts): (Value, Option<Table>)| {
            let json: serde_json::Value = lua.from_value(value).map_err(|e| {
                mlua::Error::runtime(format!(
                    "ptah.json.stringify: value is not JSON-shaped — JSON objects have \
                     string keys and arrays use consecutive 1..n indexes: {e}"
                ))
            })?;
            let indent: Option<usize> = match &opts {
                Some(t) => t.get("indent")?,
                None => None,
            };
            let encoded = match indent {
                None => serde_json::to_string(&json)
                    .map_err(|e| mlua::Error::runtime(format!("ptah.json.stringify: {e}")))?,
                Some(n) => {
                    let n = n.clamp(1, 16);
                    let spaces = vec![b' '; n];
                    let mut out = Vec::new();
                    let mut ser = serde_json::Serializer::with_formatter(
                        &mut out,
                        serde_json::ser::PrettyFormatter::with_indent(&spaces),
                    );
                    serde::Serialize::serialize(&json, &mut ser)
                        .map_err(|e| mlua::Error::runtime(format!("ptah.json.stringify: {e}")))?;
                    String::from_utf8(out)
                        .map_err(|e| mlua::Error::runtime(format!("ptah.json.stringify: {e}")))?
                }
            };
            Ok(encoded)
        })?,
    )?;
    ptah.set("json", json)?;

    // ptah.exit(code)
    let exit = lua.create_function(|lua, code: Option<i32>| {
        let state = runtime_state(lua)?;
        let code = code.unwrap_or(0);
        state.exit_code.set(Some(code));
        Err::<(), _>(mlua::Error::external(ExitSignal { code }))
    })?;
    ptah.set("exit", exit)?;

    // ptah.version (read-only)
    ptah.set("version", ptah_core::VERSION)?;
    ptah.set_readonly(true);

    let globals = lua.globals();
    globals.set("ptah", ptah)?;
    Ok(())
}

/// Build a `{ ok = true, value = v }` / `{ ok = false, error = msg }` entry.
/// Multi-value task results contribute their first value.
fn outcome_entry(lua: &Lua, res: mlua::Result<MultiValue>) -> mlua::Result<Table> {
    let entry = lua.create_table()?;
    match res {
        Ok(values) => {
            let value = values.into_iter().next().unwrap_or(Value::Nil);
            entry.set("ok", true)?;
            entry.set("value", value)?;
        }
        Err(e) => {
            entry.set("ok", false)?;
            entry.set("error", task::display_error(&e))?;
        }
    }
    Ok(entry)
}
