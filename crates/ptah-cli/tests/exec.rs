//! `ptah.exec` / `ptah.json` binding tests against a stub
//! `ProcessRunner` injected directly (the port contract: options
//! parsing, event emission, result/error shaping, no-runner posture),
//! plus the pure JSON module. Real-`/bin/sh` behavior (spawn, group
//! kill, env inheritance) lives in `tests/e2e.rs` and `tests/cli.rs`.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mlua::Lua;
use ptah::config_fs;
use ptah::render::{RenderOptions, Renderer};
use ptah::script::{self, RunConfig};
use ptah_core::events::SessionEvent;
use ptah_core::ports::{EventSink, ExecError, ExecOutcome, ProcessRunner};

// ---------------------------------------------------------------------------
// Stub runner + recording sink
// ---------------------------------------------------------------------------

/// Calls the stub recorded for assertions: `(command, timeout_ms)`.
type StubCall = (String, Option<u64>);

/// A stub runner whose script is the command string: `ok` succeeds with
/// captured output, `fail` exits 3, `hang` reports the timeout the
/// budget imposed, `spawn-fail` cannot run at all. Records every call.
#[derive(Clone, Default)]
struct StubRunner {
    calls: Arc<Mutex<Vec<StubCall>>>,
}

impl StubRunner {
    fn scripted_stdout(cmd: &str) -> &'static str {
        match cmd {
            // exec → parse integration payload
            "json" => r#"[{"n":1}]"#,
            _ => "hello",
        }
    }
}

impl ProcessRunner for StubRunner {
    fn run<'a>(
        &'a self,
        cmd: &'a str,
        timeout_ms: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<ExecOutcome, ExecError>> + Send + 'a>> {
        self.calls
            .lock()
            .unwrap()
            .push((cmd.to_string(), timeout_ms));
        Box::pin(async move {
            match cmd {
                "fail" => Ok(ExecOutcome {
                    exit_code: Some(3),
                    stdout: String::new(),
                    stderr: "boom\n".into(),
                    timed_out: false,
                }),
                "hang" => Ok(ExecOutcome {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: timeout_ms.is_some(),
                }),
                "spawn-fail" => Err(ExecError::Spawn(format!("`{cmd}`: no such file"))),
                _ => Ok(ExecOutcome {
                    exit_code: Some(0),
                    stdout: Self::scripted_stdout(cmd).into(),
                    stderr: String::new(),
                    timed_out: false,
                }),
            }
        })
    }
}

/// Sink recording every event with its label (exec lifecycle assertions).
#[derive(Clone, Default)]
struct RecordingSink {
    events: Arc<Mutex<Vec<(String, SessionEvent)>>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, label: &str, event: SessionEvent) {
        self.events.lock().unwrap().push((label.to_string(), event));
    }
    fn script_log(&self, _message: &str) {}
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tmpdir(name: &str) -> std::path::PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ptah-exec-{}-{name}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a sandboxed Lua env with `runner` injected and `sink` capturing
/// events; `runner: None` models an embedder that injects no capability.
fn exec_lua(runner: Option<Arc<dyn ProcessRunner>>, sink: Arc<dyn EventSink>) -> Lua {
    let dir = tmpdir("env");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let cfg = RunConfig {
        script_path: dir.join("main.luau"),
        invocation_dir: dir,
        registry: config_fs::from_parts(None, None).unwrap(),
        transport: Arc::new(ptah::acp::Transport::new()),
        process_runner: runner,
        interaction: ptah_core::ports::InteractionMode::Unresolved,
        shutdown: None,
        renderer: sink,
        env: std::collections::BTreeMap::new(),
    };
    script::setup_lua(&cfg).unwrap()
}

/// Evaluate `src` under a tokio LocalSet (async callbacks need it).
fn eval(lua: &Lua, src: &str) -> mlua::Result<mlua::Value> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(
        tokio::task::LocalSet::new().run_until(
            lua.load(src)
                .set_name("@exec-test.luau")
                .eval_async::<mlua::Value>(),
        ),
    )
}

// ---------------------------------------------------------------------------
// ptah.exec through the stub runner
// ---------------------------------------------------------------------------

#[test]
fn exec_success_returns_result_table() {
    let sink = Arc::new(RecordingSink::default());
    let runner = Arc::new(StubRunner::default());
    let lua = exec_lua(Some(runner.clone()), sink);
    eval(
        &lua,
        r#"
local r = ptah.exec("ok")
assert(r.exitCode == 0, "exitCode: " .. tostring(r.exitCode))
assert(r.stdout == "hello", "stdout: " .. tostring(r.stdout))
assert(r.stderr == "", "stderr: " .. tostring(r.stderr))
"#,
    )
    .unwrap();
    assert_eq!(
        runner.calls.lock().unwrap().as_slice(),
        &[("ok".into(), None)]
    );
}

#[test]
fn exec_nonzero_exit_is_data_not_error() {
    let sink = Arc::new(RecordingSink::default());
    let lua = exec_lua(Some(Arc::new(StubRunner::default())), sink);
    eval(
        &lua,
        r#"
local r = ptah.exec("fail")
assert(r.exitCode == 3, "exitCode: " .. tostring(r.exitCode))
assert(r.stderr == "boom\n", "stderr: " .. tostring(r.stderr))
"#,
    )
    .unwrap();
}

#[test]
fn exec_timeout_raises_naming_command_and_budget() {
    let sink = Arc::new(RecordingSink::default());
    let lua = exec_lua(Some(Arc::new(StubRunner::default())), sink);
    let err = eval(
        &lua,
        r#"
local ok, err = pcall(ptah.exec, "hang", { timeoutMs = 100 })
assert(not ok, "timeout must raise")
error(tostring(err), 0)
"#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("hang"), "must name the command: {msg}");
    assert!(msg.contains("100ms"), "must name the budget: {msg}");
    assert!(msg.contains("timed out"), "must say timed out: {msg}");
}

#[test]
fn exec_opts_must_be_a_table() {
    let sink = Arc::new(RecordingSink::default());
    let lua = exec_lua(Some(Arc::new(StubRunner::default())), sink);
    let err = eval(
        &lua,
        r#"
local ok, err = pcall(ptah.exec, "ok", 100)
assert(not ok, "bare number opts must be a type error")
error(tostring(err), 0)
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("opts must be a table"), "{}", err);
}

#[test]
fn exec_without_runner_raises_clearly() {
    let sink = Arc::new(RecordingSink::default());
    let lua = exec_lua(None, sink);
    let err = eval(
        &lua,
        r#"
local ok, err = pcall(ptah.exec, "ok")
assert(not ok, "no-runner must raise")
error(tostring(err), 0)
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no process runner injected"),
        "{}",
        err
    );
}

#[test]
fn exec_spawn_failure_raises_and_ends_event() {
    let sink = Arc::new(RecordingSink::default());
    let lua = exec_lua(Some(Arc::new(StubRunner::default())), sink.clone());
    let err = eval(
        &lua,
        r#"
local ok, err = pcall(ptah.exec, "spawn-fail")
assert(not ok, "spawn failure must raise")
error(tostring(err), 0)
"#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("spawn-fail"), "must name the command: {msg}");
    let events = sink.events.lock().unwrap();
    let end = events.iter().find_map(|(l, e)| match e {
        SessionEvent::ExecEnd {
            exit_code,
            timed_out,
            ..
        } => Some((l.clone(), *exit_code, *timed_out)),
        _ => None,
    });
    assert_eq!(
        end,
        Some(("exec".into(), None, false)),
        "spawn failure ends with the no-exit marker: {events:?}"
    );
}

#[test]
fn exec_emits_labeled_lifecycle_events_around_the_call() {
    let sink = Arc::new(RecordingSink::default());
    let lua = exec_lua(Some(Arc::new(StubRunner::default())), sink.clone());
    eval(&lua, r#"local r = ptah.exec("ok")"#).unwrap();
    let events = sink.events.lock().unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .map(|(label, e)| {
            let kind = match e {
                SessionEvent::ExecStart { .. } => "start",
                SessionEvent::ExecEnd { .. } => "end",
                _ => "other",
            };
            assert_eq!(
                label, "exec",
                "events must use the pseudo-label: {events:?}"
            );
            kind
        })
        .collect();
    assert_eq!(kinds, ["start", "end"], "order: {events:?}");
    // The end event carries the exit code and a duration.
    match &events[1].1 {
        SessionEvent::ExecEnd {
            exit_code,
            duration_ms,
            ..
        } => {
            assert_eq!(*exit_code, Some(0));
            assert!(*duration_ms <= 5_000, "sanity: {duration_ms}");
        }
        _ => panic!("expected ExecEnd: {events:?}"),
    }
}

#[test]
fn exec_threads_timeout_ms_to_the_runner() {
    let sink = Arc::new(RecordingSink::default());
    let runner = Arc::new(StubRunner::default());
    let lua = exec_lua(Some(runner.clone()), sink);
    eval(&lua, r#"ptah.exec("ok", { timeoutMs = 250 })"#).unwrap();
    assert_eq!(
        runner.calls.lock().unwrap().as_slice(),
        &[("ok".into(), Some(250))]
    );
    // nil timeoutMs is absent, not zero.
    eval(&lua, r#"ptah.exec("ok", { timeoutMs = nil })"#).unwrap();
    assert_eq!(
        runner.calls.lock().unwrap().last(),
        Some(&("ok".into(), None))
    );
}

// ---------------------------------------------------------------------------
// ptah.json (pure module)
// ---------------------------------------------------------------------------

#[test]
fn json_round_trip_with_indent() {
    let sink = Arc::new(Renderer::new(RenderOptions::quiet())) as Arc<dyn EventSink>;
    let lua = exec_lua(None, sink);
    eval(
        &lua,
        r#"
local v = ptah.json.parse('{"a":[1,2]}')
assert(v.a[1] == 1 and v.a[2] == 2, "parse shape")
local s = ptah.json.stringify(v, { indent = 2 })
assert(s == "{\n  \"a\": [\n    1,\n    2\n  ]\n}", "stringify: " .. s)
local compact = ptah.json.stringify(v)
assert(compact == '{"a":[1,2]}', "compact: " .. compact)
-- null decodes to nil
local n = ptah.json.parse('{"x":null}')
assert(n.x == nil, "null must be nil")
-- scalars and nested shapes round trip
assert(ptah.json.parse("true") == true)
assert(ptah.json.parse("7") == 7)
assert(ptah.json.parse('"t"') == "t")
"#,
    )
    .unwrap();
}

#[test]
fn json_malformed_input_raises_catchably() {
    let sink = Arc::new(Renderer::new(RenderOptions::quiet())) as Arc<dyn EventSink>;
    let lua = exec_lua(None, sink);
    let err = eval(
        &lua,
        r#"
local ok, err = pcall(ptah.json.parse, "{oops")
assert(not ok, "malformed must raise")
error(tostring(err), 0)
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("ptah.json.parse"), "{err}");
}

#[test]
fn json_stringify_rejects_non_string_keys() {
    let sink = Arc::new(Renderer::new(RenderOptions::quiet())) as Arc<dyn EventSink>;
    let lua = exec_lua(None, sink);
    let err = eval(
        &lua,
        r#"
local ok, err = pcall(ptah.json.stringify, { [0] = "zero" })
assert(not ok, "0-indexed keys are not JSON arrays")
error(tostring(err), 0)
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("string keys"),
        "clear message: {err}"
    );
}

#[test]
fn exec_stdout_flows_into_json_parse() {
    // The anchor workflow: deterministic command output becomes script
    // data (the stub's `json` script returns [{"n":1}]).
    let sink = Arc::new(Renderer::new(RenderOptions::quiet())) as Arc<dyn EventSink>;
    let lua = exec_lua(Some(Arc::new(StubRunner::default())), sink);
    eval(
        &lua,
        r#"
local r = ptah.exec("json")
local items = ptah.json.parse(r.stdout)
assert(#items == 1, "count: " .. tostring(#items))
assert(items[1].n == 1, "n: " .. tostring(items[1].n))
"#,
    )
    .unwrap();
}
