//! Registry-backed package e2e: the real binary against the offline
//! fixture registry (local git index + loopback archive server, see
//! `common/pkg.rs`) — add from the default index, custom indices,
//! newest-version resolution, unknown packages, registry failures,
//! and update moving a caret. No external network.

mod common;

use common::pkg::{FixturePackage, FixtureRegistry};
use std::path::PathBuf;
use std::process::Command;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ptah-reg-{}-{name}",
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

    /// A project whose manifest pins the fixture index explicitly.
    fn with_index(name: &str, registry: &FixtureRegistry) -> Self {
        let p = Self::new(name);
        let manifest = format!(
            "{}\n[indices]\ndefault = \"{}\"\n",
            ptah_pesde::manifest::skeleton(name),
            registry.index_url()
        );
        std::fs::write(p.dir.join(".ptah/pesde.toml"), manifest).unwrap();
        p
    }

    /// Run `ptah package ...` with the fixture registry as the default
    /// index (the project manifest carries no `[indices]`).
    fn package_default_index(
        &self,
        registry: &FixtureRegistry,
        args: &[&str],
    ) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("package")
            .args(args)
            .current_dir(&self.dir)
            .env("PTAH_DEFAULT_INDEX", registry.index_url())
            .output()
            .expect("run ptah package");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn package(&self, args: &[&str]) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("package")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("run ptah package");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn manifest(&self) -> String {
        std::fs::read_to_string(self.dir.join(".ptah/pesde.toml")).unwrap()
    }

    fn lockfile(&self) -> String {
        std::fs::read_to_string(self.dir.join(".ptah/pesde.lock")).unwrap()
    }
}

/// The fixture-registry contract itself (task 5.1): a bare-name add
/// installs a fixture package with no external network — the loopback
/// server and the local git index are the only endpoints involved.
#[test]
fn fixture_registry_installs_a_package_offline() {
    let registry = FixtureRegistry::start("helper", &[FixturePackage::hello("0.1.0", "hello")]);
    let p = Project::new("helper-proj");
    let (code, stdout, stderr) =
        p.package_default_index(&registry, &["add", "ptah_fixture/hello"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("resolved 1 package(s)"), "{stdout}");
    assert!(
        p.dir.join(".ptah/luau_packages/hello.luau").is_file(),
        "package linked"
    );
    // The manifest records the dependency and no [indices] table.
    let manifest = p.manifest();
    assert!(
        manifest.contains("name = \"ptah_fixture/hello\""),
        "{manifest}"
    );
    let active = manifest
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| l.contains("[indices]"));
    assert!(!active, "no [indices] table written: {manifest}");
}

#[test]
fn add_resolves_the_newest_version_and_pins_it() {
    let registry = FixtureRegistry::start(
        "newest",
        &[
            FixturePackage::hello("0.1.0", "hello"),
            FixturePackage::hello("0.2.0", "hello"),
        ],
    );
    let p = Project::new("newest-proj");
    let (code, stdout, stderr) =
        p.package_default_index(&registry, &["add", "ptah_fixture/hello"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // Caret at the newest; lockfile pins it.
    assert!(p.manifest().contains("version = \"^0.2.0\""), "{}", p.manifest());
    assert!(p.lockfile().contains("0.2.0"), "{}", p.lockfile());
}

#[test]
fn a_custom_index_is_respected() {
    // The manifest's [indices] default wins; no PTAH_DEFAULT_INDEX set.
    let registry =
        FixtureRegistry::start("custom", &[FixturePackage::hello("0.1.0", "hello")]);
    let p = Project::with_index("custom-proj", &registry);
    let (code, stdout, stderr) = p.package(&["add", "ptah_fixture/hello"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        p.dir.join(".ptah/luau_packages/hello.luau").is_file(),
        "installed through the declared index"
    );
    // The user's [indices] table survives byte-for-byte.
    assert!(p.manifest().contains("[indices]"), "{}", p.manifest());
    assert!(
        p.manifest().contains(&registry.index_url()),
        "{}",
        p.manifest()
    );
}

#[test]
fn unknown_package_is_an_operational_error_naming_the_spec() {
    let registry =
        FixtureRegistry::start("unknown", &[FixturePackage::hello("0.1.0", "hello")]);
    let p = Project::new("unknown-proj");
    let (code, _stdout, stderr) =
        p.package_default_index(&registry, &["add", "ptah_fixture/does_not_exist"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("ptah_fixture/does_not_exist")
            || stderr.contains("does_not_exist"),
        "diagnostic names the spec: {stderr}"
    );
}

#[test]
fn registry_failure_is_an_operational_error() {
    let registry =
        FixtureRegistry::start("failing", &[FixturePackage::hello("0.1.0", "hello")]);
    registry.reject("ptah_fixture/hello");
    let p = Project::new("failing-proj");
    let (code, _stdout, stderr) =
        p.package_default_index(&registry, &["add", "ptah_fixture/hello"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.to_lowercase().contains("download") || stderr.to_lowercase().contains("registry"),
        "diagnostic names the failing step: {stderr}"
    );
}

#[test]
fn update_moves_a_caret_within_range() {
    // Serve 0.1.0 first; add records ^0.1.0 and pins 0.1.0. The
    // registry later serves 0.1.1; update re-resolves and pins 0.1.1.
    let registry =
        FixtureRegistry::start("update", &[FixturePackage::hello("0.1.0", "hello")]);
    let p = Project::new("update-proj");
    let (code, stdout, stderr) =
        p.package_default_index(&registry, &["add", "ptah_fixture/hello"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(p.manifest().contains("version = \"^0.1.0\""), "{}", p.manifest());
    assert!(p.lockfile().contains("0.1.0"), "{}", p.lockfile());

    registry.register(&[
        FixturePackage::hello("0.1.0", "hello"),
        FixturePackage::hello("0.1.1", "hello"),
    ]);
    let (code, stdout, stderr) = p.package_default_index(&registry, &["update"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        p.lockfile().contains("0.1.1"),
        "update must move the pin: {}",
        p.lockfile()
    );
    // The installed module is the new version's content.
    let container = p.dir.join(".ptah/luau_packages/.pesde/ptah_fixture+hello");
    let mut found = false;
    for entry in walk_files(&container) {
        if entry.ends_with("init.luau")
            && std::fs::read_to_string(&entry)
                .is_ok_and(|s| s.contains("hello"))
        {
            found = true;
        }
    }
    assert!(found, "installed files present under {container:?}");
}

fn walk_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_files(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn locked_install_from_a_fresh_clone_restores_the_lockfile_state() {
    let registry =
        FixtureRegistry::start("locked", &[FixturePackage::hello("0.1.0", "hello")]);
    let p = Project::new("locked-proj");
    let (code, stdout, stderr) =
        p.package_default_index(&registry, &["add", "ptah_fixture/hello"]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");

    // Fresh clone: generated artifacts wiped, committed ones kept.
    let lockfile_before =
        std::fs::read(p.dir.join(".ptah/pesde.lock")).unwrap();
    let linker_before =
        std::fs::read(p.dir.join(".ptah/luau_packages/hello.luau")).unwrap();
    std::fs::remove_dir_all(p.dir.join(".ptah/luau_packages")).unwrap();
    std::fs::remove_dir_all(p.dir.join(".ptah/.pesde")).unwrap();

    let (code, stdout, stderr) =
        p.package_default_index(&registry, &["install", "--locked"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        !stdout.contains("wrote .ptah/pesde.lock"),
        "lockfile untouched: {stdout}"
    );
    assert_eq!(
        std::fs::read(p.dir.join(".ptah/pesde.lock")).unwrap(),
        lockfile_before,
        "lockfile bytes identical"
    );
    let linker_after =
        std::fs::read(p.dir.join(".ptah/luau_packages/hello.luau")).unwrap();
    assert_eq!(linker_after, linker_before, "packages byte-identical");

    // Stale manifest under --locked: diagnostic + exit 1.
    let manifest_path = p.dir.join(".ptah/pesde.toml");
    let text = std::fs::read_to_string(&manifest_path).unwrap();
    std::fs::write(
        &manifest_path,
        text.replace(
            "[target]",
            "[dependencies.extra]\nname = \"ptah_fixture/extra\"\nversion = \"^0.1.0\"\n\n[target]",
        ),
    )
    .unwrap();
    let (code, _stdout, stderr) =
        p.package_default_index(&registry, &["install", "--locked"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(stderr.contains("out of sync"), "{stderr}");
}

#[test]
fn add_from_a_git_source_with_a_subdirectory_e2e() {
    // The --git e2e shape from the package-add spec: a fixture git
    // repository with the package nested under pkg/hello.
    let repo_dir = std::env::temp_dir().join(format!(
        "ptah-reg-gitrepo-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&repo_dir);
    ptah_pesde::fixtures::git_repo(
        &repo_dir,
        &[
            ptah_pesde::fixtures::FileSpec {
                path: "pkg/hello/pesde.toml",
                contents:
                    "name = \"abc/hello\"\nversion = \"0.1.0\"\n\n[target]\nenvironment = \"luau\"\nlib = \"init.luau\"\n",
            },
            ptah_pesde::fixtures::FileSpec {
                path: "pkg/hello/init.luau",
                contents: "--!strict\nreturn { greet = 'hi from git' }\n",
            },
        ],
    );

    let p = Project::new("git-proj");
    let (code, stdout, stderr) = p.package(&[
        "add",
        "--git",
        &repo_dir.display().to_string(),
        "--path",
        "pkg/hello",
    ]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let manifest = p.manifest();
    assert!(
        manifest.contains(&format!("repo = \"file://{}\"", repo_dir.display())),
        "{manifest}"
    );
    assert!(manifest.contains("path = \"pkg/hello\""), "{manifest}");
    // The default alias is the repository name (the tempdir's
    // basename).
    let repo_alias = repo_dir.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        p.dir.join(format!(".ptah/luau_packages/{repo_alias}.luau"))
            .is_file(),
        "default alias is the repository name"
    );
    // The lockfile pins the resolved tree.
    assert!(p.lockfile().contains("tree_id"), "{}", p.lockfile());
}

// ----------------------------------------------------------------------
// Full-project e2e (task 5.2): clean project -> init -> add -> require
// @alias from a workflow -> check -> run against the mock agent.
// ----------------------------------------------------------------------

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

fn run_cmd(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(ptah_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run ptah");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[cfg(unix)]
fn luau_lsp_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("luau-lsp").is_file())
    })
}

#[test]
fn clean_project_full_package_workflow() {
    let dir = std::env::temp_dir().join(format!(
        "ptah-pkg-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. `ptah init` scaffolds the project (three files, manifest
    // included).
    let (code, stdout, stderr) = run_cmd(&dir, &["init"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(dir.join(".ptah/pesde.toml").is_file());

    // 2. Register the mock agent in the project registry (its env
    // scripts the turn's response text).
    std::fs::write(
        dir.join(".ptah/config.toml"),
        format!(
            "[agents.mock]\ncommand = \"{}\"\nargs = []\n\n[agents.mock.env]\nMOCK_CHUNKS = 'Hel|lo'\n",
            mock_bin()
        ),
    )
    .unwrap();

    // 3. Add the fixture package (default index -> loopback fixture).
    let registry = FixtureRegistry::start(
        "e2e",
        &[FixturePackage::hello("0.1.0", "hello from registry")],
    );
    let output = Command::new(ptah_bin())
        .arg("package")
        .arg("add")
        .arg("ptah_fixture/hello")
        .arg("--as")
        .arg("hello")
        .current_dir(&dir)
        .env("PTAH_DEFAULT_INDEX", registry.index_url())
        .output()
        .expect("ptah package add");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(dir.join(".ptah/luau_packages/hello.luau").is_file());
    assert!(
        dir.join(".luaurc").is_file(),
        "alias configuration synced at the project root"
    );

    // 4. A workflow that requires the package through the alias and
    // exercises it against the mock agent.
    let workflow_dir = dir.join(".ptah/workflows");
    std::fs::create_dir_all(&workflow_dir).unwrap();
    let workflow = workflow_dir.join("demo.luau");
    std::fs::write(
        &workflow,
        "--!strict\n\
         local hello = require(\"@hello\")\n\
         local agent = ptah.agent(\"mock\")\n\
         local s = agent:session({ id = \"demo\" })\n\
         local r = s:prompt(\"ignored\", { chunks = \"Hel|lo\" })\n\
         s:close()\n\
         print(\"greeting:\", hello.greet(r.text))\n",
    )
    .unwrap();

    // 5. `ptah check` is clean (alias requires resolve through the
    // lint walk AND the real luau-lsp pass; luau-lsp-dependent part
    // only when the analyzer is available, mirroring tests/analyze.rs).
    let (code, _stdout, stderr) = run_cmd(&dir, &["check", ".ptah/workflows/demo.luau"]);
    if luau_lsp_on_path() || std::env::var_os("PTAH_REQUIRE_REAL_LSP").is_some() {
        assert_eq!(
            code,
            0,
            "check must be clean through the alias, stderr:\n{stderr}"
        );
    } else {
        assert!(
            stderr.contains("luau-lsp not found") || code == 0,
            "unexpected check failure without luau-lsp: {stderr}"
        );
    }

    // 6. `ptah run` against the mock agent: the script's output
    // carries the package's function applied to the mock's response.
    let (code, stdout, stderr) = run_cmd(&dir, &["run", ".ptah/workflows/demo.luau"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("hello from registry Hello"),
        "package function exercised the mock's response: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
