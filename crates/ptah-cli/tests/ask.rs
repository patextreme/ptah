//! `ptah.ask` integration tests. Two halves, per the change's test
//! strategy:
//!
//! 1. **In-process semantics** — scripts run through `script::run` with
//!    a fake `AskProvider` injected via `RunConfig.interaction` and a
//!    recording sink: result shapes, the four distinct raises, usage
//!    errors, FIFO serialization of concurrent asks, run-end keep-alive,
//!    and teardown drop (no resolution event).
//! 2. **Binary e2e over piped stdin** — the real stdin provider through
//!    the real binary: answer flow, `/abort`, closed-stdin raise,
//!    SIGINT-during-ask, and the session-survival guarantee (an ask
//!    pending across a slow turn never restarts the agent subprocess).

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod common;

use common::PipedRun;
use ptah::script::{self, RunConfig, RunOutcome};
use ptah_core::events::SessionEvent;
use ptah_core::ports::{
    AskError, AskOutcome, AskProvider, AskRequest, EventSink, InteractionMode,
};

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ptah-ask-{}-{name}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Fakes: scripted provider + recording sink
// ---------------------------------------------------------------------------

/// The fake ask provider: records every delivered request and answers
/// from a scripted queue of (delay, outcome). Delivered calls are
/// serialized by the binding's ask lock, so queue order is call order.
struct FakeAsk {
    requests: std::sync::Mutex<Vec<AskRequest>>,
    script: std::sync::Mutex<VecDeque<(u64, Result<AskOutcome, AskError>)>>,
}

impl FakeAsk {
    fn new(script: Vec<(u64, Result<AskOutcome, AskError>)>) -> Arc<Self> {
        Arc::new(Self {
            requests: std::sync::Mutex::new(Vec::new()),
            script: std::sync::Mutex::new(script.into()),
        })
    }

    fn requests(&self) -> Vec<AskRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl AskProvider for FakeAsk {
    fn ask<'a>(
        &'a self,
        request: AskRequest,
    ) -> Pin<Box<dyn Future<Output = Result<AskOutcome, AskError>> + Send + 'a>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let (delay, outcome) = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake ask provider ran out of scripted outcomes");
            tokio::time::sleep(Duration::from_millis(delay)).await;
            outcome
        })
    }
}

/// A recording sink: ask lifecycle events and script logs, in order.
#[derive(Clone, Default)]
struct LogSink(Arc<std::sync::Mutex<Vec<String>>>);

impl LogSink {
    fn log(&self, entry: String) {
        self.0.lock().unwrap().push(entry);
    }

    fn entries(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for LogSink {
    fn emit(&self, label: &str, event: SessionEvent) {
        match event {
            SessionEvent::AskRequested { .. } => self.log(format!("{label}:requested")),
            SessionEvent::AskResolved { action, .. } => {
                self.log(format!("{label}:resolved:{}", action.as_str()));
            }
            _ => {}
        }
    }

    fn script_log(&self, message: &str) {
        self.log(format!("log:{message}"));
    }
}

/// Run one script in-process under the given interaction mode and sink,
/// returning the outcome.
fn run_ask(
    dir: &Path,
    body: &str,
    interaction: InteractionMode,
    sink: Arc<LogSink>,
    shutdown: Option<tokio::sync::watch::Receiver<i32>>,
) -> RunOutcome {
    let script = dir.join("main.luau");
    std::fs::write(&script, body).unwrap();
    let cfg = RunConfig {
        script_path: script,
        invocation_dir: dir.to_path_buf(),
        registry: ptah::config_fs::from_parts(None, None).unwrap(),
        transport: Arc::new(ptah::acp::Transport::new()),
        process_runner: None,
        interaction,
        shutdown,
        renderer: sink,
        env: BTreeMap::new(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(tokio::task::LocalSet::new().run_until(script::run(cfg)))
}

fn respond(text: &str) -> Result<AskOutcome, AskError> {
    Ok(AskOutcome::Respond {
        text: text.to_string(),
    })
}

// ---------------------------------------------------------------------------
// In-process semantics (tasks 3.2 / 3.3)
// ---------------------------------------------------------------------------

#[test]
fn ask_responds_with_result_table_and_lifecycle_events() {
    let dir = tmpdir("respond");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(0, respond("ship it"))]);
    let out = run_ask(
        &dir,
        r#"
local a = ptah.ask({ prompt = "Continue?", details = "probe output" })
assert(a.action == "respond", "action: " .. tostring(a.action))
assert(a.text == "ship it", "text: " .. tostring(a.text))
"#,
        InteractionMode::Provider(provider.clone()),
        sink.clone(),
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    // Requested before resolved, attributed `ask 1 main.luau`.
    assert_eq!(
        sink.entries(),
        vec![
            "ask 1 main.luau:requested".to_string(),
            "ask 1 main.luau:resolved:respond".to_string(),
        ],
        "{:?}",
        sink.entries()
    );
    // The provider received prompt, details, and the attribution label.
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].prompt, "Continue?");
    assert_eq!(requests[0].details.as_deref(), Some("probe output"));
    assert_eq!(requests[0].attribution, "ask 1 main.luau");
}

#[test]
fn ask_abort_is_a_normal_result_without_text() {
    let dir = tmpdir("abort");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(0, Ok(AskOutcome::Abort))]);
    let out = run_ask(
        &dir,
        r#"
local ok, a = pcall(ptah.ask, { prompt = "Continue?" })
assert(ok, "abort must not raise")
assert(a.action == "abort", "action: " .. tostring(a.action))
assert(a.text == nil, "abort carries no text: " .. tostring(a.text))
"#,
        InteractionMode::Provider(provider),
        sink.clone(),
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    assert_eq!(
        sink.entries().last().unwrap(),
        "ask 1 main.luau:resolved:abort"
    );
}

#[test]
fn ask_usage_errors_name_the_prompt_field() {
    let dir = tmpdir("usage");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(0, respond("unused"))]);
    let out = run_ask(
        &dir,
        r#"
local ok, err = pcall(ptah.ask, { details = "context" })
assert(not ok, "missing prompt must raise")
assert(tostring(err):find("prompt", 1, true), "must name prompt: " .. tostring(err))
local ok2, err2 = pcall(ptah.ask, { prompt = 42 })
assert(not ok2, "non-string prompt must raise")
assert(tostring(err2):find("prompt", 1, true), "must name prompt: " .. tostring(err2))
local ok3, err3 = pcall(ptah.ask, "just a string")
assert(not ok3, "non-table opts must raise")
assert(tostring(err3):find("prompt", 1, true) or tostring(err3):find("table", 1, true), tostring(err3))
"#,
        InteractionMode::Provider(provider),
        sink,
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn ask_raises_four_distinct_failures() {
    // Prohibited and no-provider are mode states (they raise before the
    // port is called); provider failure and end-of-input are provider
    // results. All four pcall-contained, all four distinguishable by
    // message.
    let cases: Vec<(&str, InteractionMode)> = vec![
        ("prohibited", InteractionMode::Prohibited),
        ("no-provider", InteractionMode::Unresolved),
        (
            "provider-failed",
            InteractionMode::Provider(FakeAsk::new(vec![(
                0,
                Err(AskError::Failed("backend exploded".into())),
            )])),
        ),
        (
            "end-of-input",
            InteractionMode::Provider(FakeAsk::new(vec![(
                0,
                Err(AskError::InputClosed),
            )])),
        ),
    ];
    let mut messages = Vec::new();
    for (name, mode) in &cases {
        let dir = tmpdir(&format!("raise-{name}"));
        let sink = Arc::new(LogSink::default());
        let out = run_ask(
            &dir,
            r#"
local ok, err = pcall(ptah.ask, { prompt = "q" })
assert(not ok, "must raise")
error(tostring(err), 0)
"#,
            mode.clone(),
            sink.clone(),
            None,
        );
        assert_eq!(out.code, 1, "{name}: {:?}", out.error);
        messages.push(out.error.clone().unwrap());
        // A failed ask never emits a resolution (only teardown or a
        // human answer resolves; a raise is neither).
        assert!(
            !sink.entries().iter().any(|e| e.contains("resolved")),
            "{name}: failed asks emit no resolution: {:?}",
            sink.entries()
        );
    }
    // Pairwise distinct, and each names its condition.
    for (i, a) in messages.iter().enumerate() {
        for b in &messages[i + 1..] {
            assert_ne!(a, b, "failure messages must be distinct: {messages:?}");
        }
    }
    let [prohibited, no_provider, failed, end_of_input] = &messages[..] else {
        unreachable!()
    };
    assert!(prohibited.contains("prohibited"), "{prohibited}");
    for knob in ["--ask", "PTAH_ASK", "[ask]"] {
        assert!(
            no_provider.contains(knob),
            "no-provider must name {knob}: {no_provider}"
        );
    }
    assert!(failed.contains("failed"), "{failed}");
    assert!(
        end_of_input.to_lowercase().contains("end of input"),
        "{end_of_input}"
    );
}

#[test]
fn concurrent_asks_serialize_fifo() {
    // Ask 1 is delivered slowly, ask 2 immediately — but the second is
    // prompted only after the first resolves (binding-level ask lock,
    // taken before emitting, released after the resolution).
    let dir = tmpdir("fifo");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(80, respond("first")), (0, respond("second"))]);
    let out = run_ask(
        &dir,
        r#"
local t1 = ptah.spawn(function() return ptah.ask({ prompt = "one" }) end)
local t2 = ptah.spawn(function() return ptah.ask({ prompt = "two" }) end)
local a = t1:await()
local b = t2:await()
assert(a.action == "respond" and a.text == "first", a.text)
assert(b.action == "respond" and b.text == "second", b.text)
"#,
        InteractionMode::Provider(provider),
        sink.clone(),
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    let entries = sink.entries();
    let pos = |needle: &str| {
        entries
            .iter()
            .position(|e| e.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from {entries:?}"))
    };
    assert!(pos("ask 1 main.luau:requested") < pos("ask 1 main.luau:resolved"));
    assert!(
        pos("ask 1 main.luau:resolved") < pos("ask 2 main.luau:requested"),
        "second ask must wait for the first's resolution: {entries:?}"
    );
    assert!(pos("ask 2 main.luau:requested") < pos("ask 2 main.luau:resolved"));
}

#[test]
fn pending_ask_keeps_run_alive_and_parks_only_the_caller() {
    // The main body returns while a spawned task parks in a 250ms ask:
    // the run waits for it like any outstanding task, and unrelated
    // tasks keep finishing meanwhile (blocking is per-coroutine).
    let dir = tmpdir("keepalive");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(250, respond("late answer"))]);
    let start = Instant::now();
    let out = run_ask(
        &dir,
        r#"
ptah.spawn(function() return ptah.ask({ prompt = "stuck?" }) end)
ptah.spawn(function()
    ptah.sleep(30)
    ptah.log("other task done")
end)
-- main chunk ends immediately
"#,
        InteractionMode::Provider(provider),
        sink.clone(),
        None,
    );
    let elapsed = start.elapsed();
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    assert!(
        elapsed >= Duration::from_millis(250),
        "run must wait out the pending ask: {elapsed:?}"
    );
    let entries = sink.entries();
    let pos = |needle: &str| {
        entries
            .iter()
            .position(|e| e.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from {entries:?}"))
    };
    assert!(
        pos("other task done") < pos("resolved"),
        "other work must progress while the ask is pending: {entries:?}"
    );
}

#[test]
fn ask_dropped_by_teardown_emits_no_resolution() {
    // Outer cancellation while a task parks in a never-answering ask:
    // the run tears down, exits with the signal's code, and the pending
    // ask never resolves (no resolved event — cancellation is not a
    // human outcome).
    let dir = tmpdir("teardown");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(10_000, respond("never delivered"))]);
    let (tx, rx) = tokio::sync::watch::channel(0);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let _ = tx.send(130);
    });
    let start = Instant::now();
    let out = run_ask(
        &dir,
        r#"
ptah.spawn(function() return ptah.ask({ prompt = "waiting on a human" }) end)
ptah.sleep(10_000)
"#,
        InteractionMode::Provider(provider),
        sink.clone(),
        Some(rx),
    );
    assert_eq!(out.code, 130, "error: {:?}", out.error);
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "cancel must not wait out the ask: {:?}",
        start.elapsed()
    );
    let entries = sink.entries();
    assert!(
        entries.iter().any(|e| e == "ask 1 main.luau:requested"),
        "the ask must have been issued first: {entries:?}"
    );
    assert!(
        !entries.iter().any(|e| e.contains("resolved")),
        "dropped ask must not emit a resolution: {entries:?}"
    );
}

#[test]
fn ptah_table_stays_readonly_with_ask_present() {
    let dir = tmpdir("readonly");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(0, respond("x"))]);
    let out = run_ask(
        &dir,
        r#"
assert(ptah.ask ~= nil, "ask must be present in the ptah table")
local ok = pcall(function() ptah.ask = nil end)
assert(not ok, "the ptah table must stay read-only")
"#,
        InteractionMode::Provider(provider),
        sink,
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn ask_from_a_task_body_awaits_the_result() {
    // scripting spec "Ask from a task body": spawn returns the ask
    // result through :await().
    let dir = tmpdir("task-body");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(0, respond("from task"))]);
    let out = run_ask(
        &dir,
        r#"
local t = ptah.spawn(function() return ptah.ask({ prompt = "q" }) end)
local a = t:await()
assert(a.action == "respond" and a.text == "from task", tostring(a.text))
"#,
        InteractionMode::Provider(provider),
        sink,
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn ask_from_a_parallel_callback_awaits_the_result() {
    // scripting spec: ask is callable anywhere a yield is legal —
    // parallel fan-out bodies park per item and each carries its
    // answer back through the outcome entry.
    let dir = tmpdir("parallel-body");
    let sink = Arc::new(LogSink::default());
    let provider = FakeAsk::new(vec![(0, respond("ans")), (0, respond("ans"))]);
    let out = run_ask(
        &dir,
        r#"
local results = ptah.parallel({ "a", "b" }, function(item)
    local a = ptah.ask({ prompt = "q " .. item })
    assert(a.action == "respond", a.action)
    return item .. ":" .. a.text
end)
for i, e in ipairs(results) do
    assert(e.ok, tostring(e.error))
    local want = ({ "a", "b" })[i] .. ":ans"
    assert(e.value == want, tostring(e.value))
end
"#,
        InteractionMode::Provider(provider),
        sink,
        None,
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

// ---------------------------------------------------------------------------
// Binary e2e over piped stdin (task 5.2)
// ---------------------------------------------------------------------------

// The piped harness lives in `common` (shared with the runtime-probe
// test in tests/types.rs).

/// A project with a `mock` agent registry (pinned HOME); returns
/// `(dir, script_path)`.
fn ask_project(name: &str, script_body: &str, mock_env: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let dir = tmpdir(name);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();
    let mut env_lines = String::new();
    for (k, v) in mock_env {
        // TOML literal string: env values may carry quotes (JSON).
        env_lines.push_str(&format!("{k} = '{v}'\n"));
    }
    let config = format!(
        "[agents.mock]\ncommand = \"{}\"\nargs = []\n\n[agents.mock.env]\n{}",
        mock_bin(),
        env_lines
    );
    std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();
    let script = dir.join("main.luau");
    std::fs::write(&script, script_body).unwrap();
    (dir, script)
}

#[test]
fn piped_stdin_answer_flow_responds() {
    let (dir, script) = ask_project(
        "pipe-answer",
        r#"
local a = ptah.ask({ prompt = "Continue?", details = "probe output" })
assert(a.action == "respond", "action: " .. tostring(a.action))
assert(a.text == "go ahead", "text: " .. tostring(a.text))
ptah.log("GOT:" .. a.text)
"#,
        &[],
    );
    let mut run = PipedRun::spawn(&dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Continue?");
    // The indented details line renders right after the prompt.
    run.wait_for("probe output");
    run.write_line("go ahead");
    run.wait_for("GOT:go ahead");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");
    // Resolution line carries the action; the answer text appears only
    // in the script's own log line — never in a ptah-rendered echo.
    assert!(all.contains("ask 1 main.luau: respond"), "{all}");
    let rendered_echo = all
        .lines()
        .filter(|l| l.contains("go ahead") && !l.contains("GOT:"))
        .count();
    assert_eq!(rendered_echo, 0, "renderer must not re-echo the answer: {all}");
}

#[test]
fn piped_stdin_abort_sentinel_resolves_abort() {
    let (dir, script) = ask_project(
        "pipe-abort",
        r#"
local a = ptah.ask({ prompt = "Continue?" })
assert(a.action == "abort", "action: " .. tostring(a.action))
assert(a.text == nil, "abort carries no text")
ptah.log("ABORTED")
"#,
        &[],
    );
    let mut run = PipedRun::spawn(&dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Continue?");
    run.write_line("/abort");
    run.wait_for("ABORTED");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");
    assert!(all.contains("ask 1 main.luau: abort"), "{all}");
}

#[test]
fn piped_stdin_closed_without_answer_raises_end_of_input() {
    let (dir, script) = ask_project(
        "pipe-eof",
        r#"
local ok, err = pcall(ptah.ask, { prompt = "Anyone there?" })
assert(not ok, "closed stdin must raise, not resolve")
ptah.log("EOFCAUGHT:" .. tostring(err))
"#,
        &[],
    );
    let mut run = PipedRun::spawn(&dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Anyone there?");
    run.close_stdin();
    run.wait_for("EOFCAUGHT:");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");
    let caught = all
        .lines()
        .find(|l| l.contains("EOFCAUGHT:"))
        .expect("the script must log the caught error");
    assert!(
        caught.to_lowercase().contains("end of input"),
        "must name end of input: {caught}"
    );
}

#[test]
fn sigint_during_pending_ask_exits_130_without_resolving() {
    let (dir, script) = ask_project(
        "pipe-sigint",
        r#"
local a = ptah.ask({ prompt = "Stuck?" })
-- Unreachable: SIGINT must end the run before this line ever runs.
ptah.log("RESOLVED:" .. a.action)
"#,
        &[],
    );
    let mut run = PipedRun::spawn(&dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Stuck?");
    // SIGINT (not SIGKILL): the child's own signal monitor must ride
    // the run's teardown path and exit with the shell-conventional 130.
    unsafe {
        libc::kill(run.child.id() as i32, libc::SIGINT);
    }
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 130, "stderr:\n{stderr}\nstdout:\n{all}");
    assert!(
        !all.contains("RESOLVED:"),
        "cancelled ask must never deliver a result: {all}"
    );
}

// ---------------------------------------------------------------------------
// Session survival (task 5.3): the issue's headline guarantee
// ---------------------------------------------------------------------------

/// /proc pids whose cmdline contains `needle` (the mock's unique argv
/// tag) — the witness that the agent subprocess is never restarted.
fn pids_for(needle: &str) -> Vec<i32> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if raw
            .split(|b| *b == 0)
            .any(|arg| std::str::from_utf8(arg).is_ok_and(|s| s.contains(needle)))
            && let Some(name) = entry.file_name().to_str()
            && let Ok(pid) = name.parse::<i32>()
        {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    pids
}

#[test]
fn agent_session_survives_an_ask_across_a_slow_turn() {
    // Task A parks in a slow mock turn; the main body asks and is
    // answered; A completes; the SAME session (same subprocess,
    // asserted by pid before and after the ask, session still open)
    // serves another prompt. An ask never restarts the agent.
    let tag = format!("--ptah-ask-survive-{}", std::process::id());
    let body = format!(
        r#"
local agent = ptah.agent({{ command = "{mock}", args = {{ "{tag}" }}, env = {{ MOCK_DELAY_MS = "600" }} }})
local s = agent:session({{ id = "survivor" }})
local slow = ptah.spawn(function() return s:prompt("slow turn") end)
ptah.sleep(100) -- the turn is in flight now
local a = ptah.ask({{ prompt = "Turn in flight — proceed?" }})
assert(a.action == "respond", "action: " .. tostring(a.action))
assert(a.text == "go", "text: " .. tostring(a.text))
local r = slow:await()
assert(r.stopReason == "end_turn", "in-flight turn must complete normally: " .. r.stopReason)
local again = s:prompt("still the same session")
assert(again.stopReason == "end_turn", again.stopReason)
ptah.log("SECOND-PROMPT-DONE")
ptah.sleep(1200) -- deterministic window: session held open while the test samples pids
ptah.log("SURVIVED")
s:close()
"#,
        mock = mock_bin(),
        tag = tag
    );
    let (dir, script) = ask_project("survival", &body, &[]);
    let mut run = PipedRun::spawn(&dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Turn in flight");
    let during_ask = pids_for(&tag);
    assert_eq!(
        during_ask.len(),
        1,
        "exactly one mock process during the ask: {during_ask:?}"
    );
    run.write_line("go");
    run.wait_for("SECOND-PROMPT-DONE");
    // Post-ask, post-second-prompt, session still open (the sleep holds
    // it): the same subprocess must still be the one and only mock — a
    // restart across the ask would have swapped the pid.
    let after = pids_for(&tag);
    assert_eq!(during_ask, after, "agent pid must not change across the ask");
    run.wait_for("SURVIVED");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");
}
