//! `ptah check` integration tests: drive the real binary against fixture
//! scripts with a stubbed `luau-lsp` on the child PATH. No network, no
//! real luau-lsp, no agent subprocesses — hermetic like the rest of the
//! suite.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

/// A temp project with a `.ptah/config.toml` defining one agent, `mock`.
/// HOME and XDG_CONFIG_HOME are pinned into the project dir so a
/// developer's user registry cannot leak into assertions.
struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ptah-check-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ptah")).unwrap();
        std::fs::write(
            dir.join(".ptah").join("config.toml"),
            format!("[agents.mock]\ncommand = \"{}\"\nargs = []\n", mock_bin()),
        )
        .unwrap();
        Self { dir }
    }

    fn write(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, body).unwrap();
        path
    }

    /// Run `ptah check <script>` with `path` as the child's entire PATH.
    fn check(&self, script: &Path, path: &Path, extra: &[&str]) -> (i32, String, String) {
        self.check_env(script, path, extra, &[])
    }

    /// Like [`Project::check`], plus environment entries on the ptah
    /// child (PTAH_ASK tests) and a non-terminal stdin (deterministic
    /// auto-detection: the check never reads stdin, but a test run
    /// inside a terminal must not let an inherited TTY flip the
    /// resolution).
    fn check_env(
        &self,
        script: &Path,
        path: &Path,
        extra: &[&str],
        envs: &[(&str, &str)],
    ) -> (i32, String, String) {
        let mut cmd = Command::new(ptah_bin());
        cmd.arg("check")
            .arg(script)
            .args(extra)
            .current_dir(&self.dir)
            .env("PATH", path)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("PTAH_ASK")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let output = cmd.output().expect("run ptah check");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Run `ptah run <script>` with the inherited PATH (for pre-flight
    /// tests the mock agent is resolvable via the registry's absolute
    /// command).
    fn run(&self, script: &Path) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("run")
            .arg(script)
            .current_dir(&self.dir)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("run ptah run");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// Write an executable `luau-lsp` shell-script stub into a fresh temp dir
/// and return the dir (to use as the child's PATH).
fn lsp_stub(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ptah-lspstub-{}-{name}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let stub = dir.join("luau-lsp");
    std::fs::write(&stub, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// Silent, succeeding luau-lsp stub.
fn happy_lsp() -> PathBuf {
    lsp_stub("happy", "exit 0")
}

fn clean_script(p: &Project) -> PathBuf {
    // Top-level calls that would spawn, prompt, and print if anything
    // executed: `check` must run them zero times.
    p.write(
        "main.luau",
        "--!strict\n\
         local agent = ptah.agent(\"mock\")\n\
         local util = require(\"./lib/util\")\n\
         local session = agent:session({ id = \"s1\" })\n\
         print(\"side effect:\", session.label(), util.greet(ptah.version))\n",
    );
    p.write(
        "lib/util.luau",
        "--!strict\n\
         return {\n\
         \tgreet = function(name: string): string\n\
         \t\treturn \"hello \" .. name\n\
         \tend,\n\
         }\n",
    );
    p.dir.join("main.luau")
}

// ---------------------------------------------------------------------
// 5.1 Clean path
// ---------------------------------------------------------------------

#[test]
fn clean_script_checks_green() {
    let p = Project::new("clean");
    let script = clean_script(&p);
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // No findings on stdout (the contract) and silence on success.
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn checking_executes_no_script_code() {
    // The clean fixture's top level would spawn an agent, print, and
    // label a session if anything ran; the stub luau-lsp never receives
    // a handshake and stdout stays empty (also covered by the clean
    // test; this pins the no-execution contract on its own).
    let p = Project::new("no-exec");
    let script = clean_script(&p);
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(!stdout.contains("side effect"), "stdout: {stdout}");
    assert!(!stdout.contains("spawning agent"), "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn check_with_no_script_argument_is_a_usage_error() {
    let output = Command::new(ptah_bin())
        .arg("check")
        .env("HOME", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn check_of_missing_script_exits_two() {
    let p = Project::new("missing");
    let path = p.dir.join("nope.luau");
    let (code, _stdout, stderr) = p.check(&path, &happy_lsp(), &[]);
    assert_eq!(code, 2);
    assert!(stderr.contains("script not found"), "{stderr}");
}

#[test]
fn registry_discovery_failure_exits_two() {
    // Invalid TOML in the project registry: the check could not run
    // (exit 2, error naming the cause) — distinct from findings (1).
    let p = Project::new("bad-registry");
    std::fs::write(
        p.dir.join(".ptah").join("config.toml"),
        "not valid toml [[[\n",
    )
    .unwrap();
    let script = clean_script(&p);
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("invalid project config"), "{stderr}");
}

// ---------------------------------------------------------------------
// 5.2 In-process findings
// ---------------------------------------------------------------------

#[test]
fn syntax_error_finds_positioned_finding() {
    let p = Project::new("syntax");
    let script = p.write("main.luau", "--!strict\nlocal x = {\nprint(x)\n");
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &["--no-color"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty());
    assert!(
        stderr.contains(&format!("{}:4:1:", script.display())),
        "expected positioned finding, stderr:\n{stderr}"
    );
    assert!(stderr.contains("syntax error:"), "{stderr}");
    assert!(stderr.contains("1 finding in 1 file"), "{stderr}");
    // Exactly one finding for the syntax error (no duplicate from the
    // full-moon parse of the same file).
    assert_eq!(stderr.lines().count(), 2, "{stderr}");
}

#[test]
fn unknown_literal_agent_is_a_finding() {
    let p = Project::new("unknown-agent");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal agent = ptah.agent(\"clawed\")\nreturn agent\n",
    );
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &["--no-color"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty());
    assert!(
        stderr.contains(&format!("{}:2:15:", script.display())),
        "expected call-site position, stderr:\n{stderr}"
    );
    assert!(stderr.contains("unknown agent `clawed`"), "{stderr}");
}

#[test]
fn computed_agent_name_is_not_linted() {
    let p = Project::new("computed-agent");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal name = \"clawed\"\nlocal agent = ptah.agent(name)\nreturn agent\n",
    );
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn missing_require_is_a_finding() {
    let p = Project::new("require-edges");
    let script = p.write(
        "main.luau",
        "--!strict\n\
         local a = require(\"./lib/nope\")\n\
         return a\n",
    );
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &["--no-color"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("cannot resolve require `./lib/nope`"),
        "{stderr}"
    );
    assert!(stderr.contains("1 finding in 1 file"), "{stderr}");
}

#[test]
fn cross_tree_require_is_not_a_finding() {
    // `../shared/helper` walks out of the entry's directory and both the
    // lint walk and the linter pass accept it.
    let p = Project::new("require-cross");
    p.write("shared/helper.luau", "--!strict\nreturn {}\n");
    let script = p.write(
        "workflow/main.luau",
        "--!strict\nlocal h = require(\"../shared/helper\")\nreturn h\n",
    );
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &["--no-color"]);
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn missing_strict_directive_in_module_is_a_finding() {
    let p = Project::new("strict");
    p.write("lib/util.luau", "return { greet = \"hi\" }\n");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal util = require(\"./lib/util\")\nreturn util\n",
    );
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &["--no-color"]);
    assert_eq!(code, 1, "{stderr}");
    let util = p.dir.join("lib/util.luau");
    assert!(
        stderr.contains(&format!(
            "{}:1:1: missing leading `--!strict`",
            util.display()
        )),
        "{stderr}"
    );
}

#[test]
fn findings_across_files_and_passes_are_all_reported() {
    // Entry: broken literal require (lint pass). Module: unknown agent
    // name and missing strict directive. All findings from all passes
    // are collected together, never fail-fast.
    let p = Project::new("multi");
    p.write(
        "lib/agents.luau",
        "local agent = ptah.agent(\"ghost\")\nreturn agent\n",
    );
    let script = p.write(
        "main.luau",
        "--!strict\nlocal a = require(\"./lib/nope\")\nlocal b = require(\"./lib/agents\")\nreturn a, b\n",
    );
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &["--no-color"]);
    assert_eq!(code, 1, "{stderr}");
    let entry_finding = format!("{}:2:19:", script.display());
    let module = p.dir.join("lib/agents.luau");
    let agent_finding = format!("{}:1:15: unknown agent `ghost`", module.display());
    let strict_finding = format!("{}:1:1: missing leading `--!strict`", module.display());
    assert!(stderr.contains(&entry_finding), "{stderr}");
    assert!(stderr.contains(&agent_finding), "{stderr}");
    assert!(stderr.contains(&strict_finding), "{stderr}");
    assert!(stderr.contains("3 findings in 2 files"), "{stderr}");
}

// ---------------------------------------------------------------------
// 5.3 luau-lsp stub passthrough
// ---------------------------------------------------------------------

#[test]
fn luau_lsp_findings_pass_through_raw() {
    let canned = "main.luau(1,1): TypeError: this string cannot coerce to number";
    let stub = lsp_stub("findings", &format!("echo '{canned}' >&2; exit 1"));
    let p = Project::new("lsp-findings");
    let script = clean_script(&p);
    let (code, stdout, stderr) = p.check(&script, &stub, &[]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        stderr.contains(canned),
        "expected raw passthrough of the diagnostic, stderr:\n{stderr}"
    );
}

#[test]
fn luau_lsp_warnings_do_not_fail_the_check() {
    let stub = lsp_stub(
        "warnings",
        "echo 'main.luau(2,7): LocalUnused: Variable is never used' >&2; exit 0",
    );
    let p = Project::new("lsp-warnings");
    let script = clean_script(&p);
    let (code, _stdout, _stderr) = p.check(&script, &stub, &[]);
    assert_eq!(code, 0);
}

// ---------------------------------------------------------------------
// 5.4 Missing binary
// ---------------------------------------------------------------------

#[test]
fn missing_luau_lsp_is_a_hard_error() {
    // Empty temp dir as PATH: nothing (let alone luau-lsp) is found.
    let empty = std::env::temp_dir().join(format!(
        "ptah-emptypath-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&empty).unwrap();
    let p = Project::new("no-lsp");
    let script = clean_script(&p);
    let (code, stdout, stderr) = p.check(&script, &empty, &[]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("luau-lsp not found"), "{stderr}");
}

// ---------------------------------------------------------------------
// 5.5 Run pre-flight
// ---------------------------------------------------------------------

#[test]
fn run_preflight_fails_unknown_agent_before_spawn() {
    let p = Project::new("preflight-unknown");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal agent = ptah.agent(\"clawed\")\nreturn agent\n",
    );
    let (code, stdout, stderr) = p.run(&script);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(stderr.contains("unknown agent `clawed`"), "{stderr}");
    // No agent spawned: no renderer lifecycle line on stdout.
    assert!(!stdout.contains("spawning agent"), "stdout: {stdout}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
}

#[test]
fn run_preflight_fails_broken_literal_require() {
    let p = Project::new("preflight-require");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal a = require(\"./lib/missing\")\nreturn a\n",
    );
    let (code, _stdout, stderr) = p.run(&script);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot resolve require `./lib/missing`"),
        "{stderr}"
    );
}

#[test]
fn run_preflight_lets_non_strict_scripts_through() {
    // No `--!strict`: runs exactly as before (the directive is a check
    // concern, not a run concern). A prompt turn proves full execution.
    let p = Project::new("preflight-nonstrict");
    let script = p.write(
        "main.luau",
        "local agent = ptah.agent(\"mock\")\n\
         local session = agent:session({ id = \"s1\" })\n\
         local r = session:prompt(\"hello\")\n\
         print(\"reply:\", r.text)\n",
    );
    let (code, stdout, stderr) = p.run(&script);
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(stdout.contains("reply:"), "stdout:\n{stdout}");
    assert!(stdout.contains("hello"), "stdout:\n{stdout}");
}

// ---------------------------------------------------------------------
// Interaction findings (ask capability / script-checking "Unusable
// interaction")
// ---------------------------------------------------------------------

fn ask_script(p: &Project) -> PathBuf {
    p.write(
        "main.luau",
        "--!strict\nlocal a = ptah.ask({ prompt = \"q\" })\nreturn a\n",
    )
}

#[test]
fn ask_with_none_posture_is_a_finding() {
    let p = Project::new("ask-none");
    let script = ask_script(&p);
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &["--ask=none"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains(&format!("{}:2:11:", script.display())),
        "expected a positioned finding at the call site, stderr:\n{stderr}"
    );
    assert!(stderr.contains("prohibited"), "{stderr}");
}

#[test]
fn ask_unresolvable_off_terminal_is_a_finding_naming_remedies() {
    // No --ask / PTAH_ASK / [ask], stdin not a terminal: the resolution
    // is Unresolved — a gap, reported with the three remedies.
    let p = Project::new("ask-unresolvable");
    let script = ask_script(&p);
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    for knob in ["--ask", "PTAH_ASK", "[ask]"] {
        assert!(stderr.contains(knob), "finding must name {knob}: {stderr}");
    }
}

#[test]
fn ask_with_explicit_provider_in_ci_is_clean() {
    let p = Project::new("ask-env");
    let script = ask_script(&p);
    let (code, stdout, stderr) = p.check_env(&script, &happy_lsp(), &[], &[("PTAH_ASK", "stdin")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
}

#[test]
fn check_accepts_the_ask_flag() {
    // cli spec "Check accepts the flag": --ask=stdin resolves the
    // provider for the check — no unresolvable finding — where the
    // same invocation without the flag would fail (no terminal here).
    let p = Project::new("ask-flag-check");
    let script = ask_script(&p);
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &["--ask=stdin"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
}

#[test]
fn no_ask_call_sites_ignores_the_posture() {
    // `none` + an ask-free script: no interaction finding, clean check.
    let p = Project::new("none-no-asks");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal r = ptah.exec(\"true\")\nreturn r.exitCode\n",
    );
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &["--ask=none"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
}

#[test]
fn aliased_ask_is_not_collected() {
    // `local f = ptah.ask; f(...)`: not a member call on the global —
    // documented residual; running the script raises the prohibited
    // error at call time (the runtime check is the source of truth).
    let p = Project::new("ask-alias");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal f = ptah.ask\nreturn f({ prompt = \"q\" })\n",
    );
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &["--ask=none"]);
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn registry_ask_section_feeds_the_check_resolution() {
    // The project registry's `[ask]` resolves the interaction posture
    // for the check (no flag, no env) — and a bad value fails discovery
    // (exit 2, the same class as any registry parse failure).
    let p = Project::new("ask-registry");
    std::fs::write(
        p.dir.join(".ptah").join("config.toml"),
        format!(
            "[agents.mock]\ncommand = \"{}\"\nargs = []\n\n[ask]\nprovider = \"stdin\"\n",
            mock_bin()
        ),
    )
    .unwrap();
    let script = ask_script(&p);
    let (code, stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    let p = Project::new("ask-registry-bad");
    std::fs::write(
        p.dir.join(".ptah").join("config.toml"),
        format!(
            "[agents.mock]\ncommand = \"{}\"\nargs = []\n\n[ask]\nprovider = \"stdni\"\n",
            mock_bin()
        ),
    )
    .unwrap();
    let script = ask_script(&p);
    let (code, _stdout, stderr) = p.check(&script, &happy_lsp(), &[]);
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(stderr.contains("[ask]"), "{stderr}");
}

#[test]
fn invalid_ptah_ask_env_is_a_discovery_failure_exit_2() {
    // CLI capability: an invalid PTAH_ASK value is a config-failure-class
    // error (stderr + exit 2, like registry discovery failures) on both
    // `run` and `check` — the binary-level companion to the resolution
    // unit tests.
    let p = Project::new("ask-bad-env");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal a = ptah.ask({ prompt = \"q\" })\nreturn a\n",
    );

    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .current_dir(&p.dir)
        .env("HOME", &p.dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("PTAH_ASK", "bogus")
        .stdin(Stdio::null())
        .output()
        .expect("run ptah run");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(2), "stderr:\n{stderr}");
    assert!(stderr.contains("PTAH_ASK"), "{stderr}");
    assert!(
        stderr.contains("stdin") && stderr.contains("none"),
        "names the accepted values: {stderr}"
    );

    let (code, _stdout, stderr) = p.check_env(&script, &happy_lsp(), &[], &[("PTAH_ASK", "bogus")]);
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(stderr.contains("PTAH_ASK"), "{stderr}");
}

#[test]
fn run_preflight_ask_prohibited_fails_before_spawn() {
    // cli spec "Ask with prohibited posture fails fast": the run
    // pre-flight fails before any agent subprocess spawns.
    let p = Project::new("preflight-ask-none");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal a = ptah.ask({ prompt = \"q\" })\nreturn a\n",
    );
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg("--ask=none")
        .arg(&script)
        .current_dir(&p.dir)
        .env("HOME", &p.dir)
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::null())
        .output()
        .expect("run ptah run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(stderr.contains("prohibited"), "{stderr}");
    assert!(
        stdout.is_empty(),
        "nothing renders before the run: {stdout}"
    );
}

#[test]
fn run_preflight_ask_unresolvable_off_terminal_fails_before_spawn() {
    // cli spec "Ask unresolvable without a terminal fails fast": no
    // --ask / PTAH_ASK / [ask] and a non-terminal stdin (and stdout —
    // both are pipes here) — the pre-flight fails before any agent
    // subprocess spawns, naming the selection remedies.
    let p = Project::new("preflight-ask-unresolvable");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal a = ptah.ask({ prompt = \"q\" })\nreturn a\n",
    );
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .current_dir(&p.dir)
        .env("HOME", &p.dir)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("PTAH_ASK")
        .stdin(Stdio::null())
        .output()
        .expect("run ptah run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
    for knob in ["--ask", "PTAH_ASK", "[ask]"] {
        assert!(stderr.contains(knob), "finding must name {knob}: {stderr}");
    }
    assert!(
        stdout.is_empty(),
        "nothing renders before the run: {stdout}"
    );
}

#[test]
fn run_preflight_ask_explicit_provider_in_ci_proceeds() {
    // cli spec "Ask with explicit provider passes without a terminal":
    // PTAH_ASK=stdin resolves; the run proceeds (and the script's ask
    // raises end-of-input over null stdin — the runtime check being the
    // source of truth — proving execution actually started).
    let p = Project::new("preflight-ask-env");
    let script = p.write(
        "main.luau",
        "--!strict\nlocal ok, err = pcall(ptah.ask, { prompt = \"q\" })\nassert(not ok, \"must raise over null stdin\")\nptah.log(\"ASKRAISED:\" .. tostring(err))\n",
    );
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .current_dir(&p.dir)
        .env("HOME", &p.dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("PTAH_ASK", "stdin")
        .stdin(Stdio::null())
        .output()
        .expect("run ptah run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the run must execute and exit 0 (pcall contained the raise):\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("end of input"),
        "the runtime check names the real condition: {stdout}"
    );
}
