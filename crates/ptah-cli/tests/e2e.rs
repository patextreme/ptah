//! End-to-end Luau runtime tests: scripts drive the in-repo mock agent
//! through the full `ptah` namespace.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ptah::config::Registry;
use ptah::exec::TokioProcessRunner;
use ptah::render::{RenderOptions, Renderer};
use ptah::script::{self, RunConfig, RunOutcome};
use ptah_core::ports::InteractionMode;

mod common;

fn mock_agent() -> String {
    env!("CARGO_BIN_EXE_mock-agent").to_string()
}

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ptah-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_script(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("main.luau");
    std::fs::write(&path, body).unwrap();
    path
}

fn run(script: &std::path::Path, dir: &std::path::Path) -> RunOutcome {
    run_with_registry(
        script,
        dir,
        ptah::config_fs::from_parts(None, None).unwrap(),
    )
}

fn run_with_registry(
    script: &std::path::Path,
    dir: &std::path::Path,
    registry: Registry,
) -> RunOutcome {
    let cfg = RunConfig {
        script_path: script.to_path_buf(),
        invocation_dir: dir.to_path_buf(),
        registry,
        transport: std::sync::Arc::new(ptah::acp::Transport::new()),
        // The CLI always injects the tokio runner; the suite runs the
        // same composition so `ptah.exec` behaves identically here.
        process_runner: Some(Arc::new(TokioProcessRunner::new())),
        interaction: InteractionMode::Unresolved,
        shutdown: None,
        renderer: Arc::new(Renderer::new(RenderOptions::quiet())),
        env: BTreeMap::new(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(tokio::task::LocalSet::new().run_until(script::run(cfg)))
}

#[test]
fn full_prompt_turn_from_luau() {
    let dir = tmpdir("turn");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_CHUNKS = "Hel|lo", MOCK_USAGE = "5,10,2,3" }} }})
local s = agent:session({{ id = "reviewer" }})
local r = s:prompt("ignored")
assert(r.text == "Hello", "got " .. tostring(r.text))
assert(tostring(r) == "Hello")
assert(r.stopReason == "end_turn")
assert(r.usage.input == 2 and r.usage.output == 3)
assert(s:label() == "<inline>/reviewer" or s:label():find("reviewer", 1, true))
s:close()
"#,
            mock = mock_agent()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn cross_tree_require_runs() {
    // Entry and helper live in sibling trees; the run must succeed with
    // `require("../shared/helper")` walking out of the entry directory.
    let base = std::env::temp_dir().join(format!("ptah-e2e-{}-cross-require", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("workflow")).unwrap();
    std::fs::create_dir_all(base.join("shared")).unwrap();
    std::fs::write(
        base.join("shared/helper.luau"),
        r#"return {
    drive = function(agent)
        local s = agent:session({ id = "reviewer" })
        local r = s:prompt("ignored")
        s:close()
        return r.text
    end,
}"#,
    )
    .unwrap();
    std::fs::write(
        base.join("workflow/main.luau"),
        format!(
            r#"
local helper = require("../shared/helper")
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_CHUNKS = "Hel|lo" }} }})
local text = helper.drive(agent)
assert(text == "Hello", "got " .. tostring(text))
"#,
            mock = mock_agent()
        ),
    )
    .unwrap();
    let out = run(&base.join("workflow/main.luau"), &base);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn default_session_labels() {
    let dir = tmpdir("labels");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}" }})
local a = agent:session()
local b = agent:session()
assert(a:label():match("/s1$"), a:label())
assert(b:label():match("/s2$"), b:label())
a:close()
b:close()
"#,
            mock = mock_agent()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn unknown_agent_names_the_agent() {
    let dir = tmpdir("unknown");
    let script = write_script(&dir, "local a = ptah.agent('nope')");
    let out = run(&script, &dir);
    assert_eq!(out.code, 1);
    assert!(
        out.error.as_deref().unwrap_or("").contains("nope"),
        "{:?}",
        out.error
    );
}

#[test]
fn watchdog_cancel_is_control_flow() {
    let dir = tmpdir("watchdog");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_HANG = "1" }} }})
local s = agent:session()
local a = ptah.spawn(function() return s:prompt("slow") end)
ptah.sleep(100)
s:cancel()
local r = a:await()
assert(r.stopReason == "cancelled", r.stopReason)
s:close()
"#,
            mock = mock_agent()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn timeout_is_catchable_error() {
    let dir = tmpdir("timeout");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_HANG = "1" }} }})
local s = agent:session()
local ok, err = pcall(function() return s:prompt("slow", {{ timeoutMs = 100 }}) end)
assert(not ok, "must time out")
assert(tostring(err):find("timed out", 1, true), tostring(err))
s:close()
"#,
            mock = mock_agent()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn parallel_fanout_in_order_with_cap() {
    let dir = tmpdir("parallel");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_DELAY_MS = "100" }} }})
local s = agent:session()
local results = ptah.parallel({{"a", "b", "c", "d"}}, function(item)
    local r = s:prompt(item)
    return item .. ":" .. r.text
end, {{ concurrency = 2 }})
for i, entry in ipairs(results) do
    assert(entry.ok, entry.error)
end
assert(results[1].value == "a:a", results[1].value)
assert(results[4].value == "d:d", results[4].value)
s:close()
"#,
            mock = mock_agent()
        ),
    );
    let start = Instant::now();
    let out = run(&script, &dir);
    let elapsed = start.elapsed();
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    // 4 turns, 2 at a time, 100ms each => at least 200ms total.
    assert!(
        elapsed >= Duration::from_millis(200),
        "concurrency cap not honored: {elapsed:?}"
    );
}

#[test]
fn join_contains_task_errors() {
    let dir = tmpdir("join");
    let script = write_script(
        &dir,
        r#"
local t1 = ptah.spawn(function() return 1 end)
local t2 = ptah.spawn(function() error("middle task failed", 0) end)
local t3 = ptah.spawn(function() ptah.sleep(50); return 3 end)
local outcomes = ptah.join({t1, t2, t3})
assert(outcomes[1].ok and outcomes[1].value == 1)
assert(not outcomes[2].ok and outcomes[2].error:find("middle task failed", 1, true))
assert(outcomes[3].ok and outcomes[3].value == 3)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn script_end_waits_for_pending_spawn() {
    let dir = tmpdir("pending");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_DELAY_MS = "120" }} }})
local s = agent:session()
ptah.spawn(function() s:prompt("late") end)
-- main chunk returns immediately; ptah must wait for the pending task
"#,
            mock = mock_agent()
        ),
    );
    let start = Instant::now();
    let out = run(&script, &dir);
    let elapsed = start.elapsed();
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    assert!(
        elapsed >= Duration::from_millis(110),
        "script end did not wait: {elapsed:?}"
    );
}

#[test]
fn explicit_exit_code_wins_and_tears_down() {
    let dir = tmpdir("exit");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_HANG = "1" }} }})
local s = agent:session()
ptah.spawn(function() s:prompt("pending") end)
ptah.sleep(50)
ptah.exit(3)
"#,
            mock = mock_agent()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 3, "error: {:?}", out.error);
}

#[test]
fn uncaught_error_fails_run() {
    let dir = tmpdir("error");
    let script = write_script(&dir, "error('script exploded', 0)");
    let out = run(&script, &dir);
    assert_eq!(out.code, 1);
    assert!(
        out.error
            .as_deref()
            .unwrap_or("")
            .contains("script exploded"),
        "{:?}",
        out.error
    );
}

#[test]
fn never_retrieved_task_error_fails_run() {
    let dir = tmpdir("undelivered");
    let script = write_script(
        &dir,
        r#"
ptah.spawn(function() ptah.sleep(50); error("nobody saw this", 0) end)
ptah.spawn(function() ptah.sleep(10) end)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 1);
    assert_eq!(out.undelivered_errors.len(), 1);
    assert!(
        out.undelivered_errors[0].contains("nobody saw this"),
        "{:?}",
        out.undelivered_errors
    );
}

#[test]
fn delivered_via_await_task_error_does_not_fail_run() {
    let dir = tmpdir("delivered");
    let script = write_script(
        &dir,
        r#"
local t = ptah.spawn(function() error("observed", 0) end)
pcall(function() return t:await() end)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    assert!(out.undelivered_errors.is_empty());
}

#[test]
fn default_session_cwd_is_invocation_dir() {
    // CLI spec "Session cwd defaults to invocation directory": a session
    // created without `cwd` runs in the invocation directory (the mock
    // echoes its session cwd).
    let dir = tmpdir("cwd");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_ECHO_CWD = "1" }} }})
local s = agent:session()
local r = s:prompt("where am I")
assert(r.text == "{expected}", "cwd was " .. tostring(r.text))
s:close()
"#,
            mock = mock_agent(),
            expected = dir.display()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn two_agent_calls_same_name_give_independent_factories() {
    // Scripting spec "Agent and session API": two ptah.agent calls for the
    // same name return independent factory objects (independent s1/s2
    // counters, distinct sessions).
    let dir = tmpdir("factories");
    let project_config = format!("[agents.mock]\ncommand = \"{}\"\n", mock_agent());
    let registry = ptah::config_fs::from_parts(None, Some(project_config.as_str())).unwrap();
    let script = write_script(
        &dir,
        r#"
local f1 = ptah.agent("mock")
local f2 = ptah.agent("mock")
assert(f1 ~= f2, "factories must be distinct objects")
local s1 = f1:session()
local s2 = f2:session()
assert(s1:label() == "mock/s1", s1:label())
assert(s2:label() == "mock/s1", s2:label())
assert(s1 ~= s2, "sessions must be distinct objects")
s1:close()
s2:close()
"#,
    );
    let out = run_with_registry(&script, &dir, registry);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn plain_session_outcome_has_nil_result() {
    // Scripting spec "Prompt returns a result table": the `result` field is
    // nil on sessions that declared no typed-result contract.
    let dir = tmpdir("plain-nil");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}" }})
local s = agent:session()
local r = s:prompt("hi")
assert(r.text == "hi", r.text)
assert(r.result == nil, "plain session result must be nil")
assert(r.stopReason == "end_turn")
s:close()
"#,
            mock = mock_agent()
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn log_and_version_helpers() {
    let dir = tmpdir("helpers");
    let script = write_script(
        &dir,
        r#"
ptah.log("starting")
assert(type(ptah.version) == "string" and #ptah.version > 0)
assert(ptah.sleep ~= nil and ptah.spawn ~= nil and ptah.parallel ~= nil and ptah.join ~= nil)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

// ---------------------------------------------------------------------------
// Constructor `config` rejection + setConfig sequencing
// (session-config-options capability)
// ---------------------------------------------------------------------------

/// Select `model` + dependent select `effort` options the mock advertises;
/// with `MOCK_CONFIG_DEPENDENT` a `model` set re-derives `effort` back to
/// its seeded default (`low`), modeling opencode-style dependent options.
const CONFIG_OPTIONS_JSON: &str = r#"[{"id":"model","name":"Model","type":"select","currentValue":"opus","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]},{"id":"effort","name":"Effort","type":"select","currentValue":"low","options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}]"#;

#[test]
fn constructor_config_key_rejected_before_spawn() {
    // session-config-options spec "Config key errors before spawn": the
    // key itself is the removed API (the command does not exist, so a
    // spawn would name the command instead — proving the error fires
    // first). The message teaches the migration: `setConfig` after
    // session creation, driving options first.
    let dir = tmpdir("cfg-ctor-reject");
    let script = write_script(
        &dir,
        r#"
local agent = ptah.agent({ command = "/nonexistent/ptah-test-agent" })
local ok, err = pcall(function()
    return agent:session({ config = { model = "opus" } })
end)
assert(not ok, "session() must reject the config key")
local msg = tostring(err)
assert(msg:find("config", 1, true), "must name the removed option: " .. msg)
assert(msg:find("setConfig", 1, true), "must name the replacement: " .. msg)
assert(not msg:find("/nonexistent", 1, true), "must fail before spawning: " .. msg)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn constructor_config_empty_table_rejected_identically() {
    // session-config-options spec "Empty config table errors identically":
    // the key itself signals the removed API, so an empty table raises
    // the same rejection as a populated one (pre-spawn).
    let dir = tmpdir("cfg-ctor-empty");
    let script = write_script(
        &dir,
        r#"
local agent = ptah.agent({ command = "/nonexistent/ptah-test-agent" })
local ok, err = pcall(function()
    return agent:session({ config = {} })
end)
assert(not ok, "session() must reject even an empty config table")
local msg = tostring(err)
assert(msg:find("config", 1, true), "must name the removed option: " .. msg)
assert(msg:find("setConfig", 1, true), "must name the replacement: " .. msg)
assert(not msg:find("/nonexistent", 1, true), "must fail before spawning: " .. msg)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn setconfig_sequencing_is_script_controlled() {
    // session-config-options spec "Sequencing is script-controlled": with
    // MOCK_CONFIG_DEPENDENT the mock re-derives `effort` to its default
    // (`low`) on every `model` set, so only author-ordered calls stick —
    // `model` first, `effort` after — which is exactly the sequencing the
    // removed constructor `config` table could not express.
    let dir = tmpdir("cfg-sequencing");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{
    MOCK_CONFIG_OPTIONS = '{json}',
    MOCK_CONFIG_DEPENDENT = "1",
}} }})
local s = agent:session()
s:setConfig("model", "haiku") -- re-derives effort to its default
local mid = s:configOptions()
assert(mid[2].currentValue == "low", "effort must be re-derived to its default: " .. tostring(mid[2].currentValue))
s:setConfig("effort", "high") -- applied after the model set
local options = s:configOptions()
assert(options[1].currentValue == "haiku", "model after: " .. tostring(options[1].currentValue))
assert(options[2].currentValue == "high", "effort must hold high after the ordered set: " .. tostring(options[2].currentValue))
s:close()
"#,
            mock = mock_agent(),
            json = CONFIG_OPTIONS_JSON
        ),
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn closing_one_session_keeps_same_label_survivor_in_the_run_end_sweep() {
    // Two factories for the same agent name both label their first
    // session `mock/s1` (scripting spec "Independent factories"), and
    // the session registry is the run-end teardown list — removal must
    // be by identity (pid), never by label, or closing one sibling
    // unregisters the survivor too. The survivor's mock parks on stdin
    // EOF (MOCK_EOF_LINGER), so EOF can never be the reaper here: the
    // sweep must explicitly kill the process group. (Today the driver's
    // last-handle-drop reap backstops a registry miss — this test keeps
    // that contract observable end-to-end instead of relying on it.)
    let dir = tmpdir("close-shared-label");
    let token_a = format!("--ptah-e2e-close-a-{}", std::process::id());
    let token_b = format!("--ptah-e2e-close-b-{}", std::process::id());
    let script = write_script(
        &dir,
        &format!(
            r#"
local a = ptah.agent({{ command = "{mock}", args = {{ "{token_a}" }} }})
local b = ptah.agent({{ command = "{mock}", args = {{ "{token_b}" }}, env = {{ MOCK_EOF_LINGER = "1" }} }})
local sa = a:session()
local sb = b:session()
assert(sa:label() == sb:label(), "both sessions must share a label for this regression")
sa:close()
ptah.sleep(2000)
local r = sb:prompt("hi")
assert(type(r.text) == "string", "survivor must stay usable after the sibling close")
"#,
            mock = mock_agent(),
            token_a = token_a,
            token_b = token_b
        ),
    );
    let run_dir = dir;
    let run_script = script;
    let runner = std::thread::spawn(move || run(&run_script, &run_dir));
    common::wait_for_processes(&token_a, 0, "closed session reaped by close()");
    common::wait_for_processes(&token_b, 1, "same-label survivor alive mid-run");
    assert!(
        !runner.is_finished(),
        "observation must happen while the script is still running"
    );
    let out = runner.join().unwrap();
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    // The script never closed the survivor: the run-end sweep must reap
    // it even though a same-label sibling was closed earlier.
    common::wait_for_processes(&token_b, 0, "survivor reaped by the run-end sweep");
}

// ---------------------------------------------------------------------------
// ptah.exec (shell-exec capability)
// ---------------------------------------------------------------------------

#[test]
fn exec_runs_command_and_pipeline() {
    let dir = tmpdir("exec-ok");
    let script = write_script(
        &dir,
        r#"
local r = ptah.exec("printf hello")
assert(r.exitCode == 0, "exitCode: " .. tostring(r.exitCode))
assert(r.stdout == "hello", "stdout: " .. tostring(r.stdout))
assert(r.stderr == "", "stderr: " .. tostring(r.stderr))
local p = ptah.exec("printf 'a\nb\n' | wc -l")
assert(p.exitCode == 0 and p.stdout:find("2", 1, true), "pipeline: " .. p.stdout)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn exec_nonzero_exit_returns_data() {
    let dir = tmpdir("exec-fail");
    let script = write_script(
        &dir,
        r#"
local r = ptah.exec("sh -c 'echo boom >&2; exit 3'")
assert(r.exitCode == 3, "exitCode: " .. tostring(r.exitCode))
assert(r.stdout == "", "stdout: " .. tostring(r.stdout))
assert(r.stderr == "boom\n", "stderr: " .. tostring(r.stderr))
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn exec_timeout_kills_process_group() {
    // Two sleeps under one shell (held open by `wait`): the timeout's
    // process-group kill must take the shell and both children — a
    // per-pid kill would leave one alive. The sleeps' own argv (`9871`,
    // `9872`) is what the /proc scan finds; both must be dead after the
    // raise.
    // A previously killed run may have orphaned these tags; sweep so
    // the final no-orphan assertions are about *this* run.
    common::clear_stale_tag("9871");
    common::clear_stale_tag("9872");
    let dir = tmpdir("exec-timeout");
    let script = write_script(
        &dir,
        r#"
local ok, err = pcall(ptah.exec, "sleep 9871 & sleep 9872 & wait", { timeoutMs = 300 })
assert(not ok, "timeout must raise")
local msg = tostring(err)
assert(msg:find("timed out", 1, true), msg)
assert(msg:find("300ms", 1, true), "must name the budget: " .. msg)
assert(msg:find("wait", 1, true), "must name the command: " .. msg)
"#,
    );
    let start = Instant::now();
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    // The raise came from the budget, not the full 9871s of sleep.
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "run must not wait out the sleeps: {:?}",
        start.elapsed()
    );
    // No orphans: the whole group is dead after the run.
    common::wait_for_processes("9871", 0, "first sleep dead");
    common::wait_for_processes("9872", 0, "second sleep dead");
}

#[test]
fn exec_stdin_is_eof() {
    // `cat` reads stdin until EOF; with exec's nulled stdin it must exit
    // immediately instead of hanging (or stealing ptah's own stdin).
    let dir = tmpdir("exec-stdin");
    let script = write_script(
        &dir,
        r#"
local r = ptah.exec("cat", { timeoutMs = 5000 })
assert(r.exitCode == 0, "cat must see EOF and exit: " .. tostring(r.exitCode))
assert(r.stdout == "", "no input was available")
"#,
    );
    let start = Instant::now();
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "cat must not hang: {:?}",
        start.elapsed()
    );
}

#[test]
fn spawned_agent_progresses_during_exec() {
    // shell-exec spec "Spawned agents keep progressing": the agent turn
    // (mock delay 500ms) overlaps the exec (200ms sleep); total stays
    // near the 500ms turn, far from a serialized 700ms.
    let dir = tmpdir("exec-progress");
    let script = write_script(
        &dir,
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_DELAY_MS = "500" }} }})
local s = agent:session()
local t = ptah.spawn(function() return s:prompt("ignored") end)
local r = ptah.exec("sleep 0.2")
assert(r.exitCode == 0)
local turn = t:await()
assert(turn.text == "ignored", turn.text)
s:close()
"#,
            mock = mock_agent()
        ),
    );
    let start = Instant::now();
    let out = run(&script, &dir);
    let elapsed = start.elapsed();
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    assert!(
        elapsed >= Duration::from_millis(450),
        "must still wait for the turn: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(680),
        "agent must progress during the exec (serialized would be ~700ms): {elapsed:?}"
    );
}

#[test]
fn in_flight_exec_killed_on_script_error() {
    // shell-exec spec "Script error kills running child": a spawned task
    // parks in a long exec; the main body errors and ends the run —
    // teardown kills the exec's process group before returning.
    // A previously killed run may have orphaned this tag; sweep so
    // the final no-orphan assertion is about *this* run.
    common::clear_stale_tag("9873");
    let dir = tmpdir("exec-teardown-error");
    let script = write_script(
        &dir,
        r#"
ptah.spawn(function() return ptah.exec("sleep 9873") end)
ptah.sleep(150)
error("script exploded", 0)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 1);
    assert!(out.error.unwrap().contains("script exploded"));
    common::wait_for_processes("9873", 0, "in-flight exec killed by script-error teardown");
}

#[test]
fn in_flight_exec_killed_on_ptah_exit() {
    // shell-exec spec "ptah.exit kills running child".
    // A previously killed run may have orphaned this tag; sweep so
    // the final no-orphan assertion is about *this* run.
    common::clear_stale_tag("9874");
    let dir = tmpdir("exec-teardown-exit");
    let script = write_script(
        &dir,
        r#"
ptah.spawn(function() return ptah.exec("sleep 9874") end)
ptah.sleep(150)
ptah.exit(0)
"#,
    );
    let out = run(&script, &dir);
    assert_eq!(out.code, 0, "error: {:?}", out.error);
    common::wait_for_processes("9874", 0, "in-flight exec killed by ptah.exit teardown");
}

#[test]
fn non_utf8_environment_entries_read_as_unset() {
    // The composition root captures the env snapshot via `vars_os` +
    // UTF-8 filter: a non-UTF-8 entry must not crash the run at
    // startup (`env::vars()` panics on it — design D5) and must read
    // as `nil` through `os.getenv`. Exercises the real binary so the
    // cli.rs capture is what's under test.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;

        let dir = tmpdir("env-nonutf8");
        let script = write_script(
            &dir,
            r#"
assert(os.getenv("PTAH_ENV_PROBE") == "x", "set variable must read through")
assert(os.getenv("PTAH_ENV_BAD") == nil, "non-UTF-8 entry must read as unset")
ptah.exit(0)
"#,
        );
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ptah"))
            .arg("run")
            .arg(&script)
            .current_dir(&dir)
            .env("PTAH_ENV_PROBE", "x")
            .env(
                "PTAH_ENV_BAD",
                std::ffi::OsString::from_vec(vec![0xff, 0xfe, 0xfd]),
            )
            .output()
            .expect("run ptah");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "run with a non-UTF-8 env entry must succeed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}
