//! `ptah package` integration tests: the CLI surface — argument
//! shapes, project discovery, exit-code contract, and the `.luaurc`
//! sync — against the real binary. The install flows run against
//! local fixture sources (path packages, fixture git repositories);
//! registry flows live in `package_registry.rs` with the fixture
//! index + loopback archive server. Fully offline.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

/// A temp project directory with a `.ptah/pesde.toml` manifest.
struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ptah-pkg-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ptah")).unwrap();
        std::fs::write(
            dir.join(".ptah/pesde.toml"),
            ptah_pesde::manifest::skeleton(name),
        )
        .unwrap();
        Self { dir }
    }

    /// Run a `ptah package ...` command from `from` (defaults to the
    /// project root) and return (code, stdout, stderr).
    fn package(&self, args: &[&str]) -> (i32, String, String) {
        self.package_from(&self.dir.clone(), args)
    }

    fn package_from(&self, from: &Path, args: &[&str]) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("package")
            .args(args)
            .current_dir(from)
            .output()
            .expect("run ptah package");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// A local path package (name, contents) in its own tempdir. Each
/// call creates a fresh directory (tests run in parallel and must not
/// recreate a package another test is installing from).
fn path_package(name: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "ptah-pkg-src-{}-{seq}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("pesde.toml"),
        format!(
            "name = \"abc/{name}\"\nversion = \"0.1.0\"\n\n[target]\nenvironment = \"luau\"\nlib = \"init.luau\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("init.luau"),
        "--!strict\nreturn { greet = 'hello from package' }\n",
    )
    .unwrap();
    dir
}

// ----------------------------------------------------------------------
// Usage errors (exit 2)
// ----------------------------------------------------------------------

#[test]
fn package_add_without_any_source_is_a_usage_error() {
    let p = Project::new("usage-add");
    let (code, _out, stderr) = p.package(&["add"]);
    assert_eq!(code, 2, "stderr:\n{stderr}");
}

#[test]
fn package_add_conflicting_sources_are_usage_errors() {
    let p = Project::new("usage-conflict");
    for args in [
        vec!["add", "abc/thing", "--git", "https://example.com/repo"],
        vec!["add", "abc/thing", "--path", "/tmp"],
        vec!["add", "--rev", "main"],
        vec!["add", "--git", "https://example.com/repo", "--path", "/tmp", "abc/thing"],
    ] {
        let (code, _out, stderr) = p.package(&args);
        assert_eq!(code, 2, "args {args:?}: stderr:\n{stderr}");
    }
}

#[test]
fn package_remove_unknown_alias_is_a_usage_error() {
    let p = Project::new("usage-remove");
    let (code, _out, stderr) = p.package(&["remove", "nope"]);
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(stderr.contains("nope"), "{stderr}");
}

#[test]
fn package_commands_without_a_project_exit_2() {
    let nowhere =
        std::env::temp_dir().join(format!("ptah-pkg-nowhere-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&nowhere);
    std::fs::create_dir_all(&nowhere).unwrap();
    let p = Project::new("usage-noproject");
    for args in [
        vec!["install"],
        vec!["update"],
        vec!["remove", "x"],
        vec!["add", "abc/thing"],
    ] {
        let (code, _out, stderr) = p.package_from(&nowhere, &args);
        assert_eq!(code, 2, "args {args:?}: stderr:\n{stderr}");
        assert!(stderr.contains("no ptah project found"), "{stderr}");
    }
    let _ = std::fs::remove_dir_all(&nowhere);
}

// ----------------------------------------------------------------------
// Real flows against local path sources (offline)
// ----------------------------------------------------------------------

#[test]
fn add_from_a_path_source_installs_and_syncs_the_luaurc() {
    let p = Project::new("path-add");
    let pkg = path_package("hello");

    let (code, stdout, stderr) = p.package(&["add", "--path", &pkg.display().to_string()]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // Stage lines and the alias defaulting to the directory name.
    assert!(stdout.contains("added"), "{stdout}");
    assert!(stdout.contains("resolved 1 package(s)"), "{stdout}");
    // First mutating command prints the commit/ignore guidance.
    assert!(stdout.contains("Source control:"), "{stdout}");
    for name in ["pesde.toml", "pesde.lock", ".luaurc", "luau_packages/", ".pesde/"] {
        assert!(stdout.contains(name), "guidance names {name}: {stdout}");
    }
    // The root .luaurc was created with the alias.
    let luaurc =
        std::fs::read_to_string(p.dir.join(".luaurc")).expect("root .luaurc created");
    assert!(
        luaurc.contains(&format!(
            "\"{}\": \"./.ptah/luau_packages/{}\"",
            pkg.file_name().unwrap().to_string_lossy(),
            pkg.file_name().unwrap().to_string_lossy()
        )),
        "{luaurc}"
    );
    assert!(
        p.dir.join(".ptah/luau_packages").is_dir(),
        "packages installed"
    );
    assert!(p.dir.join(".ptah/pesde.lock").is_file());
}

#[test]
fn package_install_is_idempotent_and_guidance_prints_once() {
    let p = Project::new("install-idempotent");
    let pkg = path_package("hello");
    let (code, stdout, _stderr) = p.package(&[
        "add",
        "--path",
        &pkg.display().to_string(),
        "--as",
        "hello",
    ]);
    assert_eq!(code, 0, "{stdout}");

    let (code, stdout, stderr) = p.package(&["install"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // Guidance printed once: not again on the second command.
    assert!(!stdout.contains("Source control:"), "{stdout}");
    // Lockfile unchanged: no "wrote" line.
    assert!(!stdout.contains("wrote .ptah/pesde.lock"), "{stdout}");
}

#[test]
fn package_commands_run_from_a_nested_directory() {
    let p = Project::new("nested-run");
    let pkg = path_package("hello");
    let (code, stdout, stderr) = p.package(&[
        "add",
        "--path",
        &pkg.display().to_string(),
        "--as",
        "hello",
    ]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");

    // From a nested directory: install still targets the project.
    let nested = p.dir.join(".ptah/workflows");
    std::fs::create_dir_all(&nested).unwrap();
    let (code, stdout, stderr) = p.package_from(&nested, &["install"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("resolved 1 package(s)"), "{stdout}");
}

#[test]
fn remove_uninstalls_and_cleans_the_alias() {
    let p = Project::new("remove");
    let pkg = path_package("hello");
    let (code, stdout, stderr) = p.package(&[
        "add",
        "--path",
        &pkg.display().to_string(),
        "--as",
        "hello",
    ]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");

    let (code, stdout, stderr) = p.package(&["remove", "hello"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("removed dependency `hello`"), "{stdout}");
    assert!(
        !p.dir.join(".ptah/luau_packages/hello.luau").exists(),
        "linker module removed"
    );
    // The .luaurc entry is gone too.
    let luaurc = std::fs::read_to_string(p.dir.join(".luaurc")).unwrap();
    assert!(!luaurc.contains("\"hello\""), "{luaurc}");
}

#[test]
fn locked_install_rejects_missing_and_stale_lockfiles() {
    let p = Project::new("locked");
    // No lockfile yet.
    let (code, _out, stderr) = p.package(&["install", "--locked"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(stderr.contains("lockfile is missing"), "{stderr}");

    // Install, then make the manifest stale.
    let pkg = path_package("hello");
    let (code, stdout, stderr) = p.package(&[
        "add",
        "--path",
        &pkg.display().to_string(),
        "--as",
        "hello",
    ]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    let manifest = p.dir.join(".ptah/pesde.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "[target]",
            "[dependencies.extra]\nname = \"abc/extra\"\nversion = \"^0.1.0\"\n\n[target]",
        ),
    )
    .unwrap();
    let (code, _out, stderr) = p.package(&["install", "--locked"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(stderr.contains("out of sync"), "{stderr}");
}

#[test]
fn unparseable_manifest_is_a_usage_error() {
    let p = Project::new("bad-manifest");
    std::fs::write(p.dir.join(".ptah/pesde.toml"), "not toml {{{").unwrap();
    let (code, _out, stderr) = p.package(&["install"]);
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(stderr.contains("pesde.toml"), "{stderr}");
}

#[test]
fn non_luau_target_is_rejected_with_exit_1() {
    let p = Project::new("roblox-target");
    std::fs::write(
        p.dir.join(".ptah/pesde.toml"),
        "name = \"abc/x\"\nversion = \"0.1.0\"\n\n[target]\nenvironment = \"roblox\"\n",
    )
    .unwrap();
    let (code, _out, stderr) = p.package(&["install"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(stderr.contains("roblox"), "{stderr}");
}

#[test]
fn no_install_edits_the_manifest_only() {
    let p = Project::new("no-install");
    let pkg = path_package("hello");
    let (code, stdout, stderr) = p.package(&[
        "add",
        "--path",
        &pkg.display().to_string(),
        "--as",
        "hello",
        "--no-install",
    ]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let manifest = std::fs::read_to_string(p.dir.join(".ptah/pesde.toml")).unwrap();
    assert!(manifest.contains("hello"), "{manifest}");
    assert!(
        !p.dir.join(".ptah/pesde.lock").exists(),
        "--no-install writes no lockfile"
    );
    assert!(
        !p.dir.join(".ptah/luau_packages").exists(),
        "--no-install installs nothing"
    );
}
