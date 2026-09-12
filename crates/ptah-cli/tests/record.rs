//! Run-record integration tests: a real `ptah run` against the mock agent
//! leaves `.ptah/runs/<id>/` with a `log` and a `run.json`, the record
//! tracks sessions and asks as facts arrive, its status comes from how the
//! run ended, and a record that cannot be written never fails the run.
//!
//! Complements `ptah-render`'s unit coverage of the record module
//! (id minting, the ignore file, the `run.json` model, atomic rewrites,
//! the secrets rule) with the composition-root wiring: creation, the
//! fan-out, the rewrite triggers, and the best-effort failure path.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use ptah::render::record::{RunMeta, RunStatus};

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

/// A project directory holding a `.ptah/config.toml` that names the mock
/// agent. Unique per test so records never bleed across the suite.
struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str) -> Self {
        Self::new_env(name, &[])
    }

    fn new_env(name: &str, agent_env: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("ptah-record-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ptah")).unwrap();
        let mut config = format!("[agents.mock]\ncommand = \"{}\"\nargs = []\n", mock_bin());
        if !agent_env.is_empty() {
            config.push_str("\n[agents.mock.env]\n");
            for (k, v) in agent_env {
                // TOML literal strings: env values may carry quotes (JSON).
                config.push_str(&format!("{k} = '{v}'\n"));
            }
        }
        std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();
        Self { dir }
    }

    fn script(&self, body: &str) -> PathBuf {
        let path = self.dir.join("main.luau");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn run(&self, script: &Path, flags: &[&str]) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("run")
            .arg(script)
            .args(flags)
            .current_dir(&self.dir)
            .env_remove("PTAH_ASK")
            .output()
            .expect("run ptah");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// The single record directory under `<project>/.ptah/runs/`.
fn single_record(project_dir: &Path) -> PathBuf {
    let runs = project_dir.join(".ptah/runs");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&runs)
        .unwrap_or_else(|e| panic!("no runs dir at {}: {e}", runs.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert_eq!(dirs.len(), 1, "expected exactly one record: {dirs:?}");
    dirs.pop().unwrap()
}

/// Parse the record's `run.json`.
fn read_meta(project_dir: &Path) -> RunMeta {
    let dir = single_record(project_dir);
    let raw = std::fs::read_to_string(dir.join("run.json")).expect("read run.json");
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("run.json does not parse: {e}\n{raw}"))
}

// ---------------------------------------------------------------------------
// 3.1 Record creation and the fan-out
// ---------------------------------------------------------------------------

#[test]
fn run_creates_a_log_and_run_json() {
    let project = Project::new("creates");
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );
    let (code, stdout, stderr) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");

    let dir = single_record(&project.dir);
    assert!(dir.join("log").is_file(), "no log file: {dir:?}");
    assert!(dir.join("run.json").is_file(), "no run.json: {dir:?}");
    // Self-ignoring: the sibling `.gitignore` holds `*`.
    let ignore = project.dir.join(".ptah/runs/.gitignore");
    assert!(ignore.is_file(), "no ignore file");
    assert!(std::fs::read_to_string(&ignore).unwrap().contains('*'));

    // Id shape: `yyyymmddhhmmss-<digits>`, the directory name.
    let id = dir.file_name().unwrap().to_string_lossy().into_owned();
    let (ts, suffix) = id.split_once('-').unwrap_or_else(|| panic!("no dash: {id}"));
    assert_eq!(ts.len(), 14, "{id}");
    assert!(ts.bytes().all(|b| b.is_ascii_digit()), "{id}");
    assert!(!suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()), "{id}");

    // The record is the superset of the terminal: the log carries the
    // rendered stream even though the run's own verbosity governs it.
    let log = std::fs::read_to_string(dir.join("log")).unwrap();
    assert!(log.contains("[mock/s1]"), "log missing the stream:\n{log}");
}

#[test]
fn record_log_is_the_superset_of_a_quiet_terminal() {
    let project = Project::new("quiet-superset");
    let script = project.script(
        r#"
ptah.log("script note")
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );
    let (code, stdout, stderr) = project.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    // The terminal suppresses the streaming render (`print` would still
    // pass through; this script does not print).
    assert!(!stdout.contains("[mock/"), "terminal not quiet:\n{stdout}");
    let log = std::fs::read_to_string(single_record(&project.dir).join("log")).unwrap();
    assert!(log.contains("script note"), "quiet log missing ptah.log:\n{log}");
    assert!(log.contains("[mock/s1]"), "quiet log missing the stream:\n{log}");
    assert!(!log.contains('\u{1b}'), "record log must be colorless: {log:?}");
}

#[test]
fn log_is_not_a_stdout_capture_print_and_stderr_are_excluded() {
    // run-record "The log is the rendered stream ...". Script `print`
    // bypasses the renderer (it writes process stdout through mlua) and
    // ptah's own error report is a separate channel (standard error), so
    // neither appears in the record's `log` — the record is the rendered
    // stream, not a stdout capture.
    let project = Project::new("not-a-capture");
    let script = project.script(
        r#"
print("hello-from-print")
error("boom-from-stderr", 0)
"#,
    );
    let (code, stdout, stderr) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 1, "stderr:\n{stderr}\nstdout:\n{stdout}");
    // Both channels do carry their text on the terminal/redirect sides.
    assert!(
        stdout.contains("hello-from-print"),
        "print must reach stdout:\n{stdout}"
    );
    assert!(
        stderr.contains("boom-from-stderr"),
        "the uncaught error must reach stderr:\n{stderr}"
    );

    let log = std::fs::read_to_string(single_record(&project.dir).join("log")).unwrap();
    assert!(
        !log.contains("hello-from-print"),
        "script print must be excluded from the log:\n{log}"
    );
    assert!(
        !log.contains("boom-from-stderr"),
        "ptah's stderr must be excluded from the log:\n{log}"
    );
}

// ---------------------------------------------------------------------------
// 3.2 Rewrite triggers and run end
// ---------------------------------------------------------------------------

#[test]
fn run_json_names_sessions_with_acp_ids_and_authored_shape() {
    let project = Project::new_env("sessions", &[("MOCK_CHUNKS", "hello")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );
    let (code, stdout, stderr) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");

    let meta = read_meta(&project.dir);
    assert_eq!(meta.sessions.len(), 1, "{meta:?}");
    let session = &meta.sessions[0];
    assert_eq!(session.label, "mock/s1");
    assert_eq!(session.agent, "mock");
    // Authored (pre-interpolation) invocation shape from the registry.
    assert_eq!(session.command, mock_bin());
    assert!(session.args.is_empty(), "{:?}", session.args);
    assert_eq!(session.env_keys, vec!["MOCK_CHUNKS".to_string()]);
    // The agent-assigned ACP id, taken from the structured readiness event.
    assert_eq!(session.acp_id, "mock-session-1");
    // Completed run: status, end instant, and exit code.
    assert_eq!(meta.status, RunStatus::Ok);
    assert_eq!(meta.exit_code, Some(0));
    assert!(meta.ended_at.is_some());
    assert!(meta.error.is_none());
}

#[test]
fn inline_templated_command_records_the_authored_label_and_leaks_no_value() {
    // run-record "The record pins invocation shape, never secrets": an
    // inline spec's label prefix is the *authored* command, so a templated
    // `command = "${VAR}"` cannot carry its resolved value into `run.json`
    // (the label is recorded verbatim). Regression for a leak where the
    // label was derived from the resolved command.
    let project = Project::new("inline-template-label");
    let script = project.script(
        r#"
local s = ptah.agent({ command = "${PTAH_TEST_INLINE_CMD}" }):session({ id = "x" })
s:close()
"#,
    );
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .arg("--no-color")
        .current_dir(&project.dir)
        .env_remove("PTAH_ASK")
        .env("PTAH_TEST_INLINE_CMD", mock_bin())
        .output()
        .expect("run ptah");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");

    let raw = std::fs::read_to_string(single_record(&project.dir).join("run.json")).unwrap();
    // The authored template is what the record keeps, and the resolved
    // binary path it interpolated to appears nowhere.
    assert!(raw.contains("${PTAH_TEST_INLINE_CMD}"), "{raw}");
    assert!(
        !raw.contains(mock_bin()),
        "resolved inline command leaked into run.json:\n{raw}"
    );
    let meta: RunMeta = serde_json::from_str(&raw).unwrap();
    assert_eq!(meta.sessions[0].label, "${PTAH_TEST_INLINE_CMD}/x");
    assert_eq!(meta.sessions[0].agent, "${PTAH_TEST_INLINE_CMD}");
    assert_eq!(meta.sessions[0].command, "${PTAH_TEST_INLINE_CMD}");
}

#[test]
fn second_ask_is_recorded_ordinal_2_with_its_resolution() {
    let project = Project::new("asks");
    let script = project.script(
        r#"
local a = ptah.ask({ prompt = "first?" })
local b = ptah.ask({ prompt = "second?", details = "why" })
ptah.log("OK:" .. a.text .. ":" .. b.text)
"#,
    );
    let mut run = common::PipedRun::spawn(&project.dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: first?");
    run.write_line("one");
    run.wait_for("ask 2 main.luau: second?");
    run.wait_for("why");
    run.write_line("two");
    run.wait_for("OK:one:two");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");

    let meta = read_meta(&project.dir);
    assert_eq!(meta.asks.len(), 2, "{meta:?}");
    assert_eq!(meta.asks[0].ordinal, 1);
    assert_eq!(meta.asks[1].ordinal, 2);
    assert_eq!(meta.asks[1].prompt, "second?");
    assert_eq!(meta.asks[1].details.as_deref(), Some("why"));
    assert_eq!(meta.asks[1].action.as_deref(), Some("respond"));
    assert_eq!(meta.asks[1].text.as_deref(), Some("two"));

    // run-record "Ask lines are recorded": the record's `log` carries the
    // ask prompt, the details continuation, and the resolution line — the
    // superset contract applies to asks too (they bypass `--quiet`).
    let log = std::fs::read_to_string(single_record(&project.dir).join("log")).unwrap();
    let stripped = common::strip_timestamps(&log);
    assert!(
        stripped.contains("[ptah] ask 1 main.luau: first?"),
        "log missing ask 1 prompt:\n{log}"
    );
    assert!(
        stripped.contains("[ptah] ask 2 main.luau: second?"),
        "log missing ask 2 prompt:\n{log}"
    );
    assert!(
        stripped.contains("[ptah]   why"),
        "log missing ask 2 details:\n{log}"
    );
    assert!(
        stripped.contains("[ptah] ask 2 main.luau: respond"),
        "log missing ask 2 resolution:\n{log}"
    );
}

#[test]
fn aborted_ask_is_recorded_with_action_abort_and_no_text() {
    // run-record "Aborted ask": an aborted ask is recorded with action
    // `abort` and no text. The stdin provider's `/abort` gesture drives it
    // (`crates/ptah-cli/src/ask.rs`).
    let project = Project::new("ask-abort");
    let script = project.script(
        r#"
local a = ptah.ask({ prompt = "Abort me?" })
ptah.log("ACTION:" .. a.action)
"#,
    );
    let mut run = common::PipedRun::spawn(&project.dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Abort me?");
    run.write_line("/abort");
    run.wait_for("ACTION:abort");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");

    let meta = read_meta(&project.dir);
    assert_eq!(meta.asks.len(), 1, "{meta:?}");
    assert_eq!(meta.asks[0].ordinal, 1);
    assert_eq!(meta.asks[0].prompt, "Abort me?");
    assert_eq!(meta.asks[0].action.as_deref(), Some("abort"));
    assert!(meta.asks[0].text.is_none(), "abort carries no text: {meta:?}");
}

#[cfg(unix)]
#[test]
fn first_sigint_records_cancelled_without_resolving_a_pending_ask() {
    let project = Project::new("sigint-ask");
    let script = project.script(
        r#"
local a = ptah.ask({ prompt = "Stuck?" })
-- Unreachable: SIGINT must end the run before this line ever runs.
ptah.log("RESOLVED:" .. a.action)
"#,
    );
    let mut run = common::PipedRun::spawn(&project.dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Stuck?");
    // SIGINT rides the run's teardown path (exit 130) and is reported to
    // the record as cancellation, not as a failing exit code.
    unsafe {
        libc::kill(run.child.id() as i32, libc::SIGINT);
    }
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 130, "stderr:\n{stderr}\nstdout:\n{all}");
    assert!(
        !all.contains("RESOLVED:"),
        "cancelled ask must never deliver a result: {all}"
    );

    let meta = read_meta(&project.dir);
    assert_eq!(meta.status, RunStatus::Cancelled, "{meta:?}");
    assert_eq!(meta.exit_code, Some(130));
    assert!(meta.ended_at.is_some());
    assert_eq!(meta.asks.len(), 1, "dropped ask is still recorded: {meta:?}");
    assert_eq!(meta.asks[0].prompt, "Stuck?");
    assert!(meta.asks[0].action.is_none(), "no resolution for a dropped ask");
    assert!(meta.asks[0].text.is_none());
}

#[cfg(unix)]
#[test]
fn sigkill_leaves_a_running_record_with_the_completed_log() {
    // run-record "Killed without teardown" / "An abnormally killed run keeps
    // its completed lines". SIGKILL runs no teardown, so `finish` never
    // writes: the record stays at the honest `running` state while still
    // naming the run, and the per-line-flushed `log` keeps every line it had
    // completed.
    let project = Project::new_env("sigkill", &[("MOCK_HANG", "1")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hang")
"#,
    );
    let mut run = common::PipedRun::spawn(&project.dir, &script, &["--no-color"]);
    // The prompt line renders at send time, proving the session became ready
    // and the turn is in flight before the kill.
    run.wait_for("prompt: hang");

    // The fan-out writes the terminal renderer before the record renderer,
    // so wait for the record's own `log` to catch up before the hard kill —
    // otherwise the test could race the (microseconds-later) record flush.
    let record_dir = single_record(&project.dir);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let log = std::fs::read_to_string(record_dir.join("log")).unwrap_or_default();
        if log.contains("prompt: hang") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the record's log never caught up: {log}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    unsafe {
        libc::kill(run.child.id() as i32, libc::SIGKILL);
    }
    let (code, stdout, stderr) = run.finish();
    // SIGKILL has no exit code, so the process reports none.
    assert_eq!(code, -1, "stderr:\n{stderr}\nstdout:\n{stdout}");

    let meta = read_meta(&project.dir);
    assert_eq!(meta.status, RunStatus::Running, "{meta:?}");
    assert!(meta.ended_at.is_none(), "{meta:?}");
    assert!(meta.exit_code.is_none(), "{meta:?}");
    // The start/session writes landed before the kill, so the record still
    // names the run and its started session.
    assert!(
        meta.script.ends_with("main.luau"),
        "record must still name the script: {meta:?}"
    );
    assert_eq!(meta.sessions.len(), 1, "{meta:?}");
    assert_eq!(meta.sessions[0].label, "mock/s1", "{meta:?}");

    let log = std::fs::read_to_string(record_dir.join("log")).unwrap();
    assert!(log.contains("prompt: hang"), "completed lines lost:\n{log}");
    assert!(
        log.ends_with('\n'),
        "the log must end on a completed line: {log:?}"
    );
}

#[test]
fn explicit_exit_with_a_signal_code_records_failed() {
    let project = Project::new("exit-130");
    let script = project.script("ptah.exit(130)");
    let (code, stdout, stderr) = project.run(&script, &[]);
    assert_eq!(code, 130, "stderr:\n{stderr}\nstdout:\n{stdout}");

    let meta = read_meta(&project.dir);
    // A script's own `ptah.exit(130)` is a failure, never `cancelled`:
    // cancellation comes from how the process ended, not the exit code.
    assert_eq!(meta.status, RunStatus::Failed, "{meta:?}");
    assert_eq!(meta.exit_code, Some(130));
}

#[test]
fn explicit_nonzero_exit_records_failed_with_the_code() {
    let project = Project::new("exit-3");
    let script = project.script("ptah.exit(3)");
    let (code, _, stderr) = project.run(&script, &[]);
    assert_eq!(code, 3, "stderr:\n{stderr}");
    let meta = read_meta(&project.dir);
    assert_eq!(meta.status, RunStatus::Failed);
    assert_eq!(meta.exit_code, Some(3));
}

#[test]
fn uncaught_script_error_records_failed_with_the_message() {
    let project = Project::new("script-error");
    let script = project.script("error('boom', 0)");
    let (code, _, stderr) = project.run(&script, &[]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    let meta = read_meta(&project.dir);
    assert_eq!(meta.status, RunStatus::Failed);
    assert_eq!(meta.exit_code, Some(1));
    assert!(
        meta.error.as_deref().is_some_and(|e| e.contains("boom")),
        "error not recorded: {meta:?}"
    );
}

// ---------------------------------------------------------------------------
// 3.3 Best-effort failure
// ---------------------------------------------------------------------------

#[test]
fn unwritable_record_location_warns_once_and_run_proceeds() {
    let project = Project::new_env("unwritable", &[("MOCK_CHUNKS", "hello")]);
    // A file occupies `.ptah/runs`: the record directory cannot be
    // created. Root-safe (permissions do not matter).
    std::fs::write(project.dir.join(".ptah/runs"), "occupied").unwrap();
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );

    let (code, stdout, stderr) = project.run(&script, &["--no-color"]);
    // The script runs normally: rendered output on the terminal, the
    // script's own exit code, and exactly one warning.
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert!(
        stdout.contains("hello"),
        "script output must still render:\n{stdout}"
    );
    // run-record "No line when there is no record": without a record there
    // is no run-start line — the warning above takes its place.
    assert!(
        !stdout.contains("run record"),
        "no start line may render without a record:\n{stdout}"
    );
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("run record"))
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one warning: {stderr}");
    assert!(
        warnings[0].contains(".ptah/runs"),
        "warning names the location: {}",
        warnings[0]
    );
    assert!(
        std::fs::read_to_string(project.dir.join(".ptah/runs"))
            .unwrap()
            .contains("occupied"),
        "the location must be left untouched"
    );

    // The warning survives `--quiet`: it reports a missing artifact, and
    // silence there is the failure mode the record exists to remove.
    let (code, stdout, stderr) = project.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(!stdout.contains("[mock/"), "terminal not quiet:\n{stdout}");
    assert!(
        !stdout.contains("run record"),
        "no start line may render without a record:\n{stdout}"
    );
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("run record"))
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one warning under --quiet: {stderr}");
}

#[test]
fn record_failure_does_not_change_the_scripts_exit_code() {
    let project = Project::new("unwritable-exit");
    std::fs::write(project.dir.join(".ptah/runs"), "occupied").unwrap();
    let script = project.script("ptah.exit(7)");
    let (code, _, stderr) = project.run(&script, &[]);
    // The exit code reports the script, not the record failure.
    assert_eq!(code, 7, "stderr:\n{stderr}");
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("run record"))
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one warning: {stderr}");
}

#[test]
fn mid_run_write_failure_disables_the_record_and_warns_once() {
    let project = Project::new("midrun-write");
    let script = project.script(
        r#"
local a = ptah.ask({ prompt = "Stuck?" })
ptah.log("OK:" .. a.text)
"#,
    );
    let mut run = common::PipedRun::spawn(&project.dir, &script, &["--ask=stdin"]);
    run.wait_for("ask 1 main.luau: Stuck?");

    // The ask's request rewrite must have landed before we break the
    // record: poll `run.json` until it carries the prompt, so the failure
    // we trigger below is unambiguously *mid-run* (creation succeeded).
    let record_dir = single_record(&project.dir);
    let run_json = record_dir.join("run.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let raw = std::fs::read_to_string(&run_json).unwrap_or_default();
        if raw.contains("Stuck?") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the ask's request rewrite never landed: {raw}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Break the record's next atomic rewrite: a directory occupies the
    // sibling temp path, so the temp create fails and the rename never
    // happens. Root-safe (permissions do not matter).
    std::fs::create_dir(record_dir.join("run.json.tmp")).unwrap();

    // Resolving the ask drives the failing rewrite; the run must continue
    // to completion with its own exit code and exactly one warning.
    run.write_line("answer");
    run.wait_for("OK:answer");
    let (code, stdout, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("run record"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "exactly one warning for a mid-run failure: {stderr}"
    );
    assert!(
        warnings[0].contains("run record"),
        "the warning names the record: {}",
        warnings[0]
    );

    // The record froze at its last good state (status `running`, the ask
    // requested but unresolved) and still parses — nothing partial leaked.
    let meta = read_meta(&project.dir);
    assert_eq!(
        meta.status,
        RunStatus::Running,
        "a disabled record must not write its end: {meta:?}"
    );
    assert!(meta.ended_at.is_none(), "{meta:?}");
    assert_eq!(meta.asks.len(), 1, "{meta:?}");
    assert_eq!(meta.asks[0].prompt, "Stuck?");
    assert!(
        meta.asks[0].action.is_none(),
        "the interrupted resolution must not land: {meta:?}"
    );
}

// ---------------------------------------------------------------------------
// 4.1 The run-start line
// ---------------------------------------------------------------------------

#[test]
fn run_start_line_names_the_record_at_default_verbosity() {
    let project = Project::new("start-line");
    let script = project.script("ptah.log('after the start line')\n");
    let (code, stdout, stderr) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");

    let dir = single_record(&project.dir);
    let id = dir.file_name().unwrap().to_string_lossy().into_owned();
    let want = format!("[ptah] run record: .ptah/runs/{id}");
    let stripped = common::strip_timestamps(&stdout);

    // Exactly one `ptah`-attributed line, naming the record directory
    // relative to the invocation directory, before any later output.
    assert_eq!(
        stripped.matches(&want).count(),
        1,
        "expected exactly one run-start line:\n{stdout}"
    );
    let start = stripped.find(&want).unwrap();
    let after = stripped.find("[ptah] after the start line").unwrap();
    assert!(start < after, "start line must render first:\n{stdout}");

    // The record's `log` always receives the line, even though the
    // terminal's mode governs the rest of the stream.
    let log = std::fs::read_to_string(dir.join("log")).unwrap();
    assert!(log.contains(&want), "log missing the run-start line:\n{log}");
}

#[test]
fn quiet_suppresses_the_start_line_on_the_terminal_but_not_in_the_log() {
    let project = Project::new("start-line-quiet");
    let script = project.script("ptah.log('after the start line')\n");
    let (code, stdout, stderr) = project.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");

    // `--quiet` governs the terminal alone: no start line there.
    assert!(
        !stdout.contains("run record"),
        "terminal must not show the run-start line under --quiet:\n{stdout}"
    );
    // The record still receives it (the superset contract).
    let dir = single_record(&project.dir);
    let id = dir.file_name().unwrap().to_string_lossy().into_owned();
    let log = std::fs::read_to_string(dir.join("log")).unwrap();
    assert!(
        log.contains(&format!("[ptah] run record: .ptah/runs/{id}")),
        "the record must still receive the run-start line:\n{log}"
    );
}

// ---------------------------------------------------------------------------
// 3.4 No records outside `run`
// ---------------------------------------------------------------------------

#[test]
fn preflight_failure_leaves_no_record() {
    let project = Project::new("preflight");
    // An unknown literal agent name is a pre-flight finding: the run must
    // fail before a record is ever minted.
    let script = project.script("local a = ptah.agent(\"ghost\")\n");
    let (code, _, stderr) = project.run(&script, &[]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(stderr.contains("ghost"), "{stderr}");
    assert!(
        !project.dir.join(".ptah/runs").exists(),
        "a pre-flight failure must not create a record"
    );
}

#[test]
fn non_run_commands_create_no_record() {
    // `check`
    let project = Project::new("norecord-check");
    let script = project.script("return 1\n");
    Command::new(ptah_bin())
        .arg("check")
        .arg(&script)
        .current_dir(&project.dir)
        .output()
        .expect("run ptah check");
    assert!(
        !project.dir.join(".ptah/runs").exists(),
        "check must not create a record"
    );

    // `types`
    Command::new(ptah_bin())
        .arg("types")
        .current_dir(&project.dir)
        .output()
        .expect("run ptah types");
    assert!(
        !project.dir.join(".ptah/runs").exists(),
        "types must not create a record"
    );

    // `package` (no project here: the command fails fast, and the point
    // is that no record appears on any package path)
    Command::new(ptah_bin())
        .arg("package")
        .arg("install")
        .arg("--locked")
        .current_dir(&project.dir)
        .output()
        .expect("run ptah package install");
    assert!(
        !project.dir.join(".ptah/runs").exists(),
        "package must not create a record"
    );

    // `init` in a fresh directory scaffolds `.ptah/` but no record.
    let init_dir = std::env::temp_dir().join(format!("ptah-record-{}-init", std::process::id()));
    let _ = std::fs::remove_dir_all(&init_dir);
    std::fs::create_dir_all(&init_dir).unwrap();
    Command::new(ptah_bin())
        .arg("init")
        .current_dir(&init_dir)
        .output()
        .expect("run ptah init");
    assert!(init_dir.join(".ptah").is_dir(), "init must scaffold .ptah");
    assert!(
        !init_dir.join(".ptah/runs").exists(),
        "init must not create a record"
    );
}

// ---------------------------------------------------------------------------
// 3.2 Remaining record facts
// ---------------------------------------------------------------------------

#[test]
fn only_started_sessions_appear_for_a_larger_registry() {
    // run-record "Unused agents are absent": the record has no registry
    // knowledge — it carries a session only when that session became ready,
    // so a registered-but-unused agent can neither leak nor imply it ran.
    let project = Project::new("unused-agents");
    let config_path = project.dir.join(".ptah/config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap();
    config.push_str(&format!(
        "\n[agents.unused]\ncommand = \"{}\"\nargs = []\n",
        mock_bin()
    ));
    std::fs::write(&config_path, config).unwrap();
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
ptah.log("started")
s:close()
"#,
    );
    let (code, _, stderr) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stderr:\n{stderr}");

    let meta = read_meta(&project.dir);
    assert_eq!(meta.sessions.len(), 1, "only the started session: {meta:?}");
    assert_eq!(meta.sessions[0].agent, "mock");
}

#[test]
fn undelivered_task_error_is_recorded_as_failed_with_the_message() {
    // run-record "Undelivered task error": the script finishes while a
    // spawned task's error was never observed, and the record carries the
    // failure and that task's message (mirrors `e2e`'s undelivered case).
    let project = Project::new("undelivered-record");
    let script = project.script(
        r#"
ptah.spawn(function() ptah.sleep(50); error("nobody saw this", 0) end)
ptah.spawn(function() ptah.sleep(10) end)
"#,
    );
    let (code, _, stderr) = project.run(&script, &[]);
    assert_eq!(code, 1, "stderr:\n{stderr}");

    let meta = read_meta(&project.dir);
    assert_eq!(meta.status, RunStatus::Failed, "{meta:?}");
    assert_eq!(meta.exit_code, Some(1));
    assert!(
        meta.error
            .as_deref()
            .is_some_and(|e| e.contains("nobody saw this")),
        "undelivered error not recorded: {meta:?}"
    );
}

// ---------------------------------------------------------------------------
// The self-ignoring record in a real checkout
// ---------------------------------------------------------------------------

/// Run `git` in `dir` with a synthetic identity (no global config reliance),
/// panicking on a non-zero exit.
fn git(dir: &Path, args: &[&str]) {
    let _ = git_stdout(dir, args);
}

/// Like [`git`] but returns stdout.
fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=ptah@example.invalid",
            "-c",
            "user.name=ptah",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn record_dir_stays_out_of_a_tracked_git_commit() {
    // run-record "Records stay out of commits": `.ptah/` is legitimately a
    // tracked directory, so a workflow's routine `git add -A` must stage no
    // file from `.ptah/runs/` — the self-ignoring `.gitignore` is the
    // control, and this is its end-to-end witness.
    let project = Project::new("gitignore-commits");
    let script = project.script("ptah.log('done')\n");

    git(&project.dir, &["init", "-q"]);
    git(&project.dir, &["add", ".ptah/config.toml"]);
    git(&project.dir, &["commit", "-q", "-m", "init"]);

    let (code, _, stderr) = project.run(&script, &[]);
    assert_eq!(code, 0, "stderr:\n{stderr}");
    // A record exists, so the negative assertion below is meaningful.
    let record = single_record(&project.dir);
    assert!(record.join("run.json").is_file(), "no record: {record:?}");

    git(&project.dir, &["add", "-A"]);
    let staged = git_stdout(&project.dir, &["status", "--porcelain"]);
    assert!(
        !staged.contains(".ptah/runs"),
        "a record leaked into the index:\n{staged}"
    );
}
