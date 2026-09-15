//! `ptah init` e2e: the real binary scaffolds `./.ptah` in a tempdir —
//! exactly four files, definitions byte-identical to `ptah types`
//! stdout, split existing-file semantics (configs skipped, definitions
//! synced: created / updated / up to date), a managed ignore section
//! (created / appended / updated / up to date) in a user-owned
//! `.ptah/.gitignore`, a valid package-manifest skeleton, hints on every
//! run, and a clean failure when the target can't be written.

use std::path::Path;
use std::process::Command;

/// The managed ignore section `ptah init` writes — pinned here
/// independently of the binary's constant.
const MANAGED_SECTION: &str = "\
# >>> ptah (managed section; `ptah init` refreshes it)
/luau_packages/
/.pesde/
# <<< ptah
";

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
fn fresh_init_creates_exactly_four_files_with_hints() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");

    let config = dir.path().join(".ptah").join("config.toml");
    let defs = dir.path().join(".ptah").join("ptah.d.luau");
    let manifest = dir.path().join(".ptah").join("pesde.toml");
    let ignore = dir.path().join(".ptah").join(".gitignore");
    assert!(config.is_file(), "config.toml not created");
    assert!(defs.is_file(), "ptah.d.luau not created");
    assert!(manifest.is_file(), "pesde.toml not created");
    assert!(ignore.is_file(), ".gitignore not created");
    assert!(
        stdout.contains("created: .ptah/config.toml"),
        "created line missing: {stdout}"
    );
    assert!(
        stdout.contains("created: .ptah/pesde.toml"),
        "created line missing: {stdout}"
    );
    assert!(
        stdout.contains("created: .ptah/ptah.d.luau"),
        "created line missing: {stdout}"
    );
    assert!(
        stdout.contains("created: .ptah/.gitignore"),
        "created line missing: {stdout}"
    );
    assert!(
        stdout.contains("Next steps"),
        "hints must print on a fresh run: {stdout}"
    );

    // The created ignore file is exactly the marked section: anchored
    // rules for the two generated package paths, and no runs/ rule
    // (runs/ keeps its own enclave).
    let ignore_bytes = std::fs::read_to_string(&ignore).unwrap();
    assert_eq!(ignore_bytes, MANAGED_SECTION, "ignore file must be exactly the section");
    assert!(ignore_bytes.contains("/luau_packages/\n"), "{ignore_bytes}");
    assert!(ignore_bytes.contains("/.pesde/\n"), "{ignore_bytes}");
    assert!(!ignore_bytes.contains("runs"), "no runs/ rule: {ignore_bytes}");

    // Exactly four files inside .ptah, nothing else anywhere under
    // dir: no starter script, no editor or Luau configuration, and
    // nothing package-related beyond the manifest (no lockfile, no
    // luau_packages/, no root .luaurc — init stays offline and
    // installs nothing).
    let mut entries: Vec<String> = std::fs::read_dir(dir.path().join(".ptah"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![".gitignore", "config.toml", "pesde.toml", "ptah.d.luau"]
    );
    let top: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(top, vec![".ptah"]);
}

#[tokio::test]
async fn manifest_skeleton_is_a_valid_private_luau_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // Parses through pesde's own deserialization via the adapter, in
    // the project it will be used from.
    let project = ptah_pesde::PackageProject::open(dir.path());
    let manifest = ptah_pesde::manifest::deser_manifest(&project)
        .await
        .expect("skeleton parses");
    assert!(manifest.private);
    assert_eq!(manifest.target.kind().to_string(), "luau");
    assert!(manifest.indices.is_empty(), "no indices in skeleton");
    assert!(
        manifest.dependencies.is_empty()
            && manifest.peer_dependencies.is_empty()
            && manifest.dev_dependencies.is_empty(),
        "no dependencies"
    );
    // The name is components/<sanitized directory name>.
    let dirname = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(
        manifest.name.to_string(),
        format!(
            "components/{}",
            ptah_pesde::manifest::sanitize_name_segment(&dirname)
        )
    );
}

#[test]
fn partial_scaffold_with_configs_completes() {
    // config.toml + pesde.toml exist, definitions missing: the defs
    // file is created and the configs are neither modified nor
    // clobbered.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    let user_config = "[agents.custom]\ncommand = \"my-agent\"\n";
    std::fs::write(dir.path().join(".ptah").join("config.toml"), user_config).unwrap();
    let user_manifest = "name = \"mine/thing\"\nversion = \"9.9.9\"\n\n[target]\nenvironment = \"luau\"\n";
    std::fs::write(dir.path().join(".ptah").join("pesde.toml"), user_manifest).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("skipped (exists): .ptah/config.toml"),
        "{stdout}"
    );
    assert!(
        stdout.contains("skipped (exists): .ptah/pesde.toml"),
        "{stdout}"
    );
    assert!(
        stdout.contains("created: .ptah/ptah.d.luau"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".ptah").join("config.toml")).unwrap(),
        user_config
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".ptah").join("pesde.toml")).unwrap(),
        user_manifest,
        "existing manifest must survive untouched"
    );
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

    let first_manifest =
        std::fs::read(dir.path().join(".ptah").join("pesde.toml")).unwrap();
    let first_ignore =
        std::fs::read(dir.path().join(".ptah").join(".gitignore")).unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("skipped (exists): .ptah/config.toml"),
        "config skip line missing: {stdout}"
    );
    assert!(
        stdout.contains("skipped (exists): .ptah/pesde.toml"),
        "manifest skip line missing: {stdout}"
    );
    assert!(
        stdout.contains("up to date: .ptah/ptah.d.luau"),
        "definitions up-to-date line missing: {stdout}"
    );
    assert!(
        stdout.contains("up to date: .ptah/.gitignore"),
        "ignore up-to-date line missing: {stdout}"
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
        std::fs::read(dir.path().join(".ptah").join("pesde.toml")).unwrap(),
        first_manifest,
        "re-run must leave pesde.toml byte-identical"
    );
    assert_eq!(
        std::fs::read(dir.path().join(".ptah").join("ptah.d.luau")).unwrap(),
        first_defs,
        "re-run must leave an up-to-date ptah.d.luau byte-identical"
    );
    assert_eq!(
        std::fs::read(dir.path().join(".ptah").join(".gitignore")).unwrap(),
        first_ignore,
        "re-run must leave a current .ptah/.gitignore byte-identical"
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

#[test]
fn unwritable_gitignore_fails_cleanly() {
    // `.ptah/.gitignore` exists as a *directory*: reading it fails, the
    // error is printed, and init exits 1 like every other per-file
    // failure — no warn-and-continue (the file is protection).
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah").join(".gitignore")).unwrap();
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 1, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("error: cannot write .ptah/.gitignore"),
        "expected the per-file gitignore error on stderr: {stderr}"
    );
}

#[test]
fn unmarked_ignore_file_gains_the_section() {
    // A hand-rolled `.ptah/.gitignore` with user rules and no ptah
    // markers: the section is appended, every preceding byte is
    // unchanged, and the line reads `appended:`.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    let user_rules = "# my rules\n*.log\n";
    let ignore = dir.path().join(".ptah").join(".gitignore");
    std::fs::write(&ignore, user_rules).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("appended: .ptah/.gitignore"),
        "append line missing: {stdout}"
    );
    let after = std::fs::read_to_string(&ignore).unwrap();
    assert!(
        after.starts_with(user_rules),
        "every preceding byte must be unchanged: {after:?}"
    );
    assert_eq!(
        after,
        format!("{user_rules}\n{MANAGED_SECTION}"),
        "section appended after exactly one blank line"
    );
}

#[test]
fn marked_section_is_refreshed() {
    // Markers whose between-content differs from the current section
    // (an older binary's rules): only the between bytes change,
    // everything outside is untouched, and the line reads `updated:`.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    let before = "# user before\n";
    let after = "# user after\n";
    let opening = "# >>> ptah (managed section; `ptah init` refreshes it)";
    let closing = "# <<< ptah";
    let stale = format!("{before}{opening}\n/old_stale_rule/\n{closing}\n{after}");
    let ignore = dir.path().join(".ptah").join(".gitignore");
    std::fs::write(&ignore, &stale).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("updated: .ptah/.gitignore"),
        "update line missing: {stdout}"
    );
    let expected = format!(
        "{before}{opening}\n/luau_packages/\n/.pesde/\n{closing}\n{after}"
    );
    assert_eq!(std::fs::read_to_string(&ignore).unwrap(), expected);
}

#[test]
fn user_content_outside_markers_is_preserved() {
    // User rules both before the opening marker and after the closing
    // marker: both regions are byte-for-byte unchanged and the section
    // is current.
    let dir = tempfile::tempdir().unwrap();
    let opening = "# >>> ptah (managed section; `ptah init` refreshes it)";
    let closing = "# <<< ptah";
    let before = "# before the section\n*.tmp\n";
    let after = "\n# after the section\nbuild/\n";
    // A stale between-region forces the refresh path.
    let content = format!("{before}{opening}\n/old_stale_rule/\n{closing}{after}");
    std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
    let ignore = dir.path().join(".ptah").join(".gitignore");
    std::fs::write(&ignore, &content).unwrap();

    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let refreshed = std::fs::read_to_string(&ignore).unwrap();
    assert!(refreshed.starts_with(before), "before-region changed: {refreshed:?}");
    assert!(refreshed.ends_with(after), "after-region changed: {refreshed:?}");
    assert!(
        refreshed.contains(MANAGED_SECTION.trim_end()),
        "the section must be current: {refreshed:?}"
    );
}

#[test]
fn init_writes_ignore_file_without_git() {
    // The write is unconditional — no git probe. A directory with no
    // `.git` still gets the managed section.
    let dir = tempfile::tempdir().unwrap();
    assert!(
        !dir.path().join(".git").exists(),
        "fixture must not be a git repository"
    );
    let (code, stdout, stderr) = init(dir.path());
    assert_eq!(code, 0, "exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let ignore = dir.path().join(".ptah").join(".gitignore");
    assert!(ignore.is_file(), ".gitignore must be created without .git");
    assert_eq!(std::fs::read_to_string(&ignore).unwrap(), MANAGED_SECTION);
}
