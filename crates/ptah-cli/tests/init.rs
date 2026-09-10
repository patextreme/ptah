//! `ptah init` e2e: the real binary scaffolds `./.ptah` in a tempdir —
//! exactly two files, definitions byte-identical to `ptah types`
//! stdout, split existing-file semantics (config skipped, definitions
//! synced: created / updated / up to date), hints on every run, and a
//! clean failure when the target can't be written.

use std::path::Path;
use std::process::Command;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

/// Run `ptah init` in `dir`; return (exit code, stdout, stderr).
fn init(dir: &Path) -> (i32, String, String) {
    let out = Command::new(ptah_bin())
        .arg("init")
        .current_dir(dir)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `ptah types` stdout — the current emit the definitions are synced
/// against.
fn types_stdout() -> Vec<u8> {
    let out = Command::new(ptah_bin()).arg("types").output().unwrap();
    assert!(out.status.success(), "`ptah types` failed");
    out.stdout
}

#[test]
fn fresh_init_creates_exactly_both_files_with_hints() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");

    let config = dir.path().join(".ptah").join("config.toml");
    let defs = dir.path().join(".ptah").join("ptah.d.luau");
    assert!(config.is_file(), "config.toml not created");
    assert!(defs.is_file(), "ptah.d.luau not created");
    assert!(
        stdout.contains("created: .ptah/config.toml"),
        "created line missing: {stdout}"
    );
    assert!(
        stdout.contains("created: .ptah/ptah.d.luau"),
        "created line missing: {stdout}"
    );
    assert!(
        stdout.contains("Next steps"),
        "hints must print on a fresh run: {stdout}"
    );

    // Exactly two files inside .ptah, nothing else anywhere under dir:
    // no starter script, no editor or Luau configuration.
    let mut entries: Vec<String> = std::fs::read_dir(dir.path().join(".ptah"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["config.toml", "ptah.d.luau"]);
    let top: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(top, vec![".ptah"]);
}

#[test]
fn written_definitions_are_byte_identical_to_types_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let types_out = Command::new(ptah_bin()).arg("types").output().unwrap();
    assert!(types_out.status.success());
    let written = std::fs::read(dir.path().join(".ptah").join("ptah.d.luau")).unwrap();
    assert_eq!(
        written, types_out.stdout,
        "init's ptah.d.luau must be byte-identical to `ptah types` stdout"
    );
}

#[test]
fn written_skeleton_parses_as_an_empty_registry() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let skeleton = std::fs::read_to_string(dir.path().join(".ptah").join("config.toml")).unwrap();
    let registry = ptah::config_fs::from_parts(None, Some(&skeleton))
        .unwrap_or_else(|e| panic!("skeleton must parse: {e}"));
    assert!(
        registry.agent_names().is_empty(),
        "fresh skeleton must register no agents"
    );
}

#[test]
fn rerunning_init_reports_skips_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let first_config = std::fs::read(dir.path().join(".ptah").join("config.toml")).unwrap();
    let first_defs = std::fs::read(dir.path().join(".ptah").join("ptah.d.luau")).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("skipped (exists): .ptah/config.toml"),
        "config skip line missing: {stdout}"
    );
    assert!(
        stdout.contains("up to date: .ptah/ptah.d.luau"),
        "definitions up-to-date line missing: {stdout}"
    );
    assert!(
        stdout.contains("Next steps"),
        "hints must print on every run, skipped or not: {stdout}"
    );
    assert_eq!(
        std::fs::read(dir.path().join(".ptah").join("config.toml")).unwrap(),
        first_config,
        "re-run must leave config.toml byte-identical"
    );
    assert_eq!(
        std::fs::read(dir.path().join(".ptah").join("ptah.d.luau")).unwrap(),
        first_defs,
        "re-run must leave an up-to-date ptah.d.luau byte-identical"
    );
}

#[test]
fn stale_definitions_are_updated_with_version_arrow() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    // A fabricated older emit: a genuine ptah header naming an older
    // version plus a body that differs from the current one.
    std::fs::write(
        dir.path().join(".ptah").join("ptah.d.luau"),
        "-- ptah 0.0.1 type definitions\n-- stale body\n",
    )
    .unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(
        std::fs::read(dir.path().join(".ptah").join("ptah.d.luau")).unwrap(),
        types_stdout(),
        "stale definitions must be overwritten with the current emit"
    );
    assert!(stdout.contains("updated"), "update line missing: {stdout}");
    assert!(
        stdout.contains("(0.0.1 -> "),
        "version arrow must carry the parsed old version: {stdout}"
    );
}

#[test]
fn modified_or_foreign_definitions_are_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    // Differing content whose first line is not a ptah header — a
    // hand-edited or foreign file, overwritten all the same.
    std::fs::write(
        dir.path().join(".ptah").join("ptah.d.luau"),
        "--!strict\n-- hand-edited definitions\n",
    )
    .unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(
        std::fs::read(dir.path().join(".ptah").join("ptah.d.luau")).unwrap(),
        types_stdout(),
        "modified definitions must be overwritten with the current emit"
    );
    assert!(
        stdout.lines().any(|l| l == "updated: .ptah/ptah.d.luau"),
        "update line must carry no version suffix: {stdout}"
    );
}

#[test]
fn source_layout_definitions_report_up_to_date() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    // The repo-root scenario (design D2): the file laid out like the
    // repository's own source definitions — `ptah types` stdout minus
    // its header line. It must report current, never be rewritten
    // with a prepended header.
    let emitted = types_stdout();
    let body = emitted
        .splitn(2, |&b: &u8| b == b'\n')
        .nth(1)
        .expect("emit has a header line")
        .to_vec();
    let defs = dir.path().join(".ptah").join("ptah.d.luau");
    std::fs::write(&defs, &body).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("up to date: .ptah/ptah.d.luau"),
        "source-layout definitions must report up to date: {stdout}"
    );
    assert_eq!(
        std::fs::read(&defs).unwrap(),
        body,
        "source-layout definitions must be left byte-identical (no prepended header)"
    );
}

#[test]
fn preexisting_config_survives_while_missing_defs_are_created() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    let user_config = "[agents.custom]\ncommand = \"my-agent\"\nargs = [\"--fast\"]\n";
    std::fs::write(dir.path().join(".ptah").join("config.toml"), user_config).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("skipped (exists): .ptah/config.toml"),
        "existing config must be reported skipped: {stdout}"
    );
    assert!(
        stdout.contains("created: .ptah/ptah.d.luau"),
        "missing defs must still be created: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".ptah").join("config.toml")).unwrap(),
        user_config,
        "existing config must survive untouched"
    );
    // The surviving user config still resolves its own agent.
    let registry = ptah::config_fs::from_parts(None, Some(user_config)).unwrap();
    assert_eq!(registry.agent_names(), vec!["custom".to_string()]);
}

#[test]
fn unwritable_target_fails_cleanly() {
    // A *file* named .ptah makes create_dir_all fail: error on stderr,
    // exit 1, nothing else created.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".ptah"), "i am a file, not a directory").unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 1, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("error"),
        "expected an error on stderr: {stderr}"
    );
    let top: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(top, vec![".ptah"], "failure must not create anything else");
}
