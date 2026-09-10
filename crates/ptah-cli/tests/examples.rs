//! The bundled examples run green against the mock agent (CI parity for
//! task 8.1): each example is executed via the real binary with a generated
//! project registry mapping `demo` to the mock agent.

use std::path::PathBuf;
use std::process::Command;

mod common;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

fn project(example: &str, agent_env: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ptah-examples-{}-{example}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();
    let mut config = format!(
        "[agents.demo]\ncommand = \"{}\"\nargs = []\n\n[agents.demo.env]\n",
        mock_bin()
    );
    for (k, v) in agent_env {
        // TOML literal string: env values may carry double quotes (JSON).
        config.push_str(&format!("{k} = '{v}'\n"));
    }
    std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();
    dir
}

fn run_example(example: &str, agent_env: &[(&str, &str)]) {
    run_example_with_ptah_env(example, agent_env, &[]);
}

/// Like [`run_example`], plus environment entries set on the ptah
/// process itself — the snapshot `os.getenv` reads. Distinct from
/// `agent_env`, which lands in the registry's `[agents.demo.env]` and
/// shapes the agent subprocess, not ptah.
fn run_example_with_ptah_env(example: &str, agent_env: &[(&str, &str)], ptah_env: &[(&str, &str)]) {
    let dir = project(example, agent_env);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(example);
    let mut cmd = Command::new(ptah_bin());
    cmd.arg("run").arg(&script).current_dir(&dir);
    for (k, v) in ptah_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run ptah");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{example} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Run one example through the piped-stdin harness with an explicit
/// `PTAH_ASK=stdin` provider and feed it one answer line — the ask
/// examples' entry point (prompts render, answers go in over the pipe).
/// Returns the captured stdout so callers can assert on rendered lines.
fn run_example_piped(example: &str, agent_env: &[(&str, &str)], answer: &str) -> String {
    let dir = project(example, agent_env);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(example);
    let mut run = common::PipedRun::spawn_env(&dir, &script, &[], &[("PTAH_ASK", "stdin")]);
    run.wait_for("ask 1 ");
    run.write_line(answer);
    let (code, stdout, stderr) = run.finish();
    assert!(
        code == 0,
        "{example} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
}

#[test]
fn example_sequential_review() {
    run_example("sequential_review.luau", &[]);
}

#[test]
fn example_fanout() {
    run_example("fanout.luau", &[]);
}

#[test]
fn example_watchdog() {
    run_example("watchdog.luau", &[("MOCK_HANG", "1")]);
}

#[test]
fn example_typed_results() {
    run_example(
        "typed_results.luau",
        &[("MOCK_SUBMIT", r#"{"verdict":"approve","score":8}"#)],
    );
}

#[test]
fn example_workflow_1_shared_helper() {
    // Cross-tree require: the entry requires ../shared/helper from a
    // sibling directory of its own tree.
    run_example("workflow-1/main.luau", &[]);
}

#[test]
fn example_workflow_2_shared_helper() {
    run_example("workflow-2/main.luau", &[]);
}

#[test]
fn example_model_fanout() {
    // The mock advertises a `model` select option and echoes its current
    // value in each reply, so the two sessions provably run under the two
    // models the example sets.
    run_example(
        "model-fanout.luau",
        &[
            (
                "MOCK_CONFIG_OPTIONS",
                r#"[{"id":"model","name":"Model","type":"select","currentValue":"sonnet","options":[{"value":"sonnet","name":"Sonnet"},{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]}]"#,
            ),
            ("MOCK_CONFIG_ECHO", "model"),
        ],
    );
}

#[test]
fn example_env() {
    // PTAH_EXAMPLE_* ride the ptah process env (os.getenv's snapshot);
    // PTAH_EXAMPLE_REVIEWER and PTAH_EXAMPLE_EXTRA stay unset to
    // exercise the fallback / empty-distinguishing branches.
    run_example_with_ptah_env(
        "env.luau",
        &[],
        &[
            ("PTAH_EXAMPLE_MODEL", "haiku"),
            ("PTAH_EXAMPLE_VERBOSE", "1"),
        ],
    );
}

#[test]
fn example_exec_pipeline() {
    // git may or may not be present in the project dir (it is not a
    // repo in the test sandbox): both the git path and the printf
    // fallback must carry the example.
    run_example("exec_pipeline.luau", &[]);
}

#[test]
fn example_ask() {
    // The ask example behind the piped-stdin harness: an explicit
    // PTAH_ASK=stdin provider, one answer line, the mock reviewer turn
    // replies with the prompt text (so the answer must come back in it).
    let stdout = run_example_piped("ask.luau", &[], "naming conventions");
    // The mock echoes the prompt text back, so the piped answer must
    // surface in the logged reply — proof it fed the agent turn.
    assert!(
        stdout.contains("focusing on: naming conventions"),
        "answer must propagate into the agent turn:\n{stdout}"
    );
}

#[test]
fn example_ask_prohibited_by_none_fails_the_preflight() {
    // Design D8 item 5's negative path: the ask example under
    // `--ask=none` fails the run pre-flight with the prohibited finding
    // (exit 1, no spawn; the `.output()` harness needs no stdin).
    let dir = project("ask-none", &[]);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join("ask.luau");
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg("--ask=none")
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run ptah");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(stderr.contains("prohibited"), "{stderr}");
    assert!(
        stdout.is_empty(),
        "nothing renders before the run: {stdout}"
    );
}
