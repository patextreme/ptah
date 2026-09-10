//! Real-analyzer contract tests for the type definitions — the static
//! counterpart to the runtime probe in `tests/types.rs`. The rest of the
//! suite stubs `luau-lsp` (see `tests/check.rs`) to stay hermetic; these
//! tests instead drive `ptah check` with the *real* analyzer, so the
//! binary's embedded definitions are what's under test, and assert the
//! type-definitions capability's scenarios from both sides: strict
//! scripts on the current surface analyze clean, and each documented
//! misuse reports a diagnostic naming the promised type.
//!
//! Gated on `luau-lsp` being on PATH — always true in the nix dev shell
//! and in the sandbox, where `checks.ptah-tests` injects the binary and
//! sets `PTAH_REQUIRE_REAL_LSP=1` so a silent skip cannot pass CI.
//! Plain `cargo test` elsewhere skips with a notice. Fully offline.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

/// A temp project with a `.ptah/config.toml` defining one agent, `mock`.
/// HOME is pinned into the project dir so a developer's user registry
/// cannot leak into the checks.
struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ptah-analyze-{}-{name}-{}",
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

    fn write(&self, body: &str) -> PathBuf {
        let path = self.dir.join("main.luau");
        std::fs::write(&path, body).unwrap();
        path
    }

    /// Run `ptah check <script>` with `path` as the child's entire PATH
    /// (the real luau-lsp's directory — ptah discovers the analyzer by
    /// PATH, exactly like the stub tests in check.rs).
    fn check(&self, script: &Path, path: &Path) -> (i32, String, String) {
        self.check_env(script, path, &[])
    }

    /// Like [`Project::check`] plus environment entries on the ptah
    /// child (e.g. `PTAH_ASK=stdin` so ask scripts resolve a provider
    /// without a terminal).
    fn check_env(
        &self,
        script: &Path,
        path: &Path,
        envs: &[(&str, &str)],
    ) -> (i32, String, String) {
        let mut cmd = Command::new(ptah_bin());
        cmd.arg("check")
            .arg(script)
            .current_dir(&self.dir)
            .env("PATH", path)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("PTAH_ASK")
            .stdin(Stdio::null());
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
}

/// which-style scan for an executable `luau-lsp` on PATH (the same rule
/// ptah itself uses to find the analyzer).
#[cfg(unix)]
fn luau_lsp_on_path() -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join("luau-lsp");
        std::fs::metadata(&candidate)
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .then_some(candidate)
    })
}

#[test]
#[cfg(unix)]
fn real_luau_lsp_definitions_contract() {
    let Some(lsp) = luau_lsp_on_path() else {
        if std::env::var_os("PTAH_REQUIRE_REAL_LSP").is_some() {
            panic!("PTAH_REQUIRE_REAL_LSP is set but luau-lsp is not on PATH");
        }
        eprintln!("skipping: luau-lsp not on PATH (run inside `nix develop`)");
        return;
    };
    let lsp_dir = lsp.parent().expect("luau-lsp path has a parent");

    // type-definitions "Constructor config type-checks": the
    // `SessionOptions` type declares no `config` field, and analysis
    // does NOT flag the excess key (a known table-literal analyzer
    // residual) — running the script instead raises the pre-spawn
    // rejection (pinned in tests/e2e.rs). The residual stays documented
    // so nobody rediscovers the missing static signal as a bug.
    let p = Project::new("excess-config-key");
    let script = p.write(
        "--!strict\n\
         local agent = ptah.agent(\"mock\")\n\
         local s = agent:session({ config = { model = \"opus\" } })\n\
         s:close()\n",
    );
    let (code, stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        !stderr.contains("TypeError"),
        "excess option keys are a known analyzer residual — no diagnostic expected, stderr:\n{stderr}"
    );

    // type-definitions "Wrong setConfig value type".
    let p = Project::new("bad-setconfig");
    let script = p.write(
        "--!strict\n\
         local agent = ptah.agent(\"mock\")\n\
         local s = agent:session()\n\
         s:setConfig(\"model\", 42)\n",
    );
    let (code, _stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("boolean | string"),
        "diagnostic must name the accepted value types, stderr:\n{stderr}"
    );

    // type-definitions "Typo in result field" / "Typed-result surface
    // type-checks" (rejecting side): an invented prompt-outcome field
    // reports a type error naming the result table type.
    let p = Project::new("bad-field");
    let script = p.write(
        "--!strict\n\
         local agent = ptah.agent(\"mock\")\n\
         local s = agent:session()\n\
         local r = s:prompt(\"hi\")\n\
         print(r.txt)\n",
    );
    let (code, _stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("Key 'txt' not found in table 'PromptResult'"),
        "diagnostic must name the result table type, stderr:\n{stderr}"
    );

    // Accepting sides of "Typed-result surface type-checks" and
    // "Outcome narrowing": one strict script using `resultSchema` +
    // `r.result` and branching on a locally-bound parallel outcome
    // analyzes with zero type errors. (The local binding is
    // load-bearing: narrowing does not apply through repeated index
    // expressions like `outcomes[1].ok` — the spec words the scenario
    // as binding the result to a local for exactly this reason.)
    let p = Project::new("happy");
    let script = p.write(
        "--!strict\n\
         local agent = ptah.agent(\"mock\")\n\
         local s = agent:session({\n\
         \tresultSchema = { type = \"object\" },\n\
         })\n\
         local r = s:prompt(\"hi\")\n\
         print(r.result)\n\
         local outcomes = ptah.parallel({ 1, 2 }, function(item)\n\
         \treturn item * 2\n\
         end)\n\
         local entry = outcomes[1]\n\
         if entry.ok then\n\
         \tprint(entry.value)\n\
         else\n\
         \tprint(entry.error)\n\
         end\n\
         s:close()\n",
    );
    let (code, stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        !stderr.contains("TypeError"),
        "happy-path script must analyze clean, stderr:\n{stderr}"
    );

    // type-definitions "Exec result fields type-check" / "Exec options
    // type-checks" / "JSON module type-checks" (shell-exec surface):
    // a strict exec+json script analyzes clean, and each documented
    // misuse reports a diagnostic naming the promised type.
    let p = Project::new("exec-json-happy");
    let script = p.write(
        "--!strict\n\
         local r = ptah.exec(\"true\", { timeoutMs = 100 })\n\
         print(r.exitCode, r.stdout, r.stderr)\n\
         local v = ptah.json.parse('[{\"n\":1}]')\n\
         print(ptah.json.stringify(v, { indent = 2 }))\n",
    );
    let (code, stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        !stderr.contains("TypeError"),
        "exec+json script must analyze clean, stderr:\n{stderr}"
    );

    // "Exec result fields type-check" (rejecting side): `r.out` names
    // the exec result table type.
    let p = Project::new("bad-exec-field");
    let script = p.write(
        "--!strict\n\
         local r = ptah.exec(\"true\")\n\
         print(r.exitCode, r.stdout, r.stderr, r.out)\n",
    );
    let (code, _stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("Key 'out' not found in table 'ExecResult'"),
        "diagnostic must name the exec result type, stderr:\n{stderr}"
    );

    // "Exec options type-check" (rejecting side): a bare-number opts
    // argument reports the ExecOptions type.
    let p = Project::new("bad-exec-opts");
    let script = p.write("--!strict\n ptah.exec(\"true\", 100)\n");
    let (code, _stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("but got 'number'") && stderr.contains("ExecOptions"),
        "diagnostic must name the ExecOptions type, stderr:\n{stderr}"
    );

    // "JSON module type-checks" (rejecting side): an invented module
    // member names the Json table type.
    let p = Project::new("bad-json-member");
    let script = p.write(
        "--!strict\n\
         local v = ptah.json.parse(\"1\")\n\
         ptah.json.load(v)\n",
    );
    let (code, _stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("Key 'load' not found in table 'Json'"),
        "diagnostic must name the Json module type, stderr:\n{stderr}"
    );

    // type-definitions "Ask options type-check" (accepting side) and
    // "Ask result narrows on action": a strict ask script branching on
    // the bound result analyzes clean on the respond arm.
    let p = Project::new("ask-happy");
    let script = p.write(
        "--!strict\n\
         local a = ptah.ask({ prompt = \"q\", details = \"d\" })\n\
         if a.action == \"respond\" then\n\
         \tprint(a.text)\n\
         else\n\
         \tprint(a.action)\n\
         end\n",
    );
    let (code, stdout, stderr) = p.check_env(&script, lsp_dir, &[("PTAH_ASK", "stdin")]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        !stderr.contains("TypeError"),
        "narrowed ask script must analyze clean, stderr:\n{stderr}"
    );

    // "Ask result narrows on action" (rejecting side): the abort arm
    // has no `text` — the diagnostic names the ask result type.
    let p = Project::new("ask-narrow");
    let script = p.write(
        "--!strict\n\
         local a = ptah.ask({ prompt = \"q\" })\n\
         if a.action == \"respond\" then\n\
         \tprint(a.text)\n\
         else\n\
         \tprint(a.text)\n\
         end\n",
    );
    let (code, _stdout, stderr) = p.check_env(&script, lsp_dir, &[("PTAH_ASK", "stdin")]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    // The analyzer narrows to the abort arm and reports the missing key
    // on that arm's shape (`{ action: "abort" }`).
    assert!(
        stderr.contains("Key 'text' not found") && stderr.contains("abort"),
        "diagnostic must name the abort arm's missing `text`, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("text"),
        "diagnostic must point at the `text` read, stderr:\n{stderr}"
    );

    // "Ask options type-check" (rejecting side): a call missing the
    // required `prompt` reports a type error naming the field.
    let p = Project::new("ask-missing-prompt");
    let script = p.write("--!strict\n ptah.ask({ details = \"d\" })\n");
    let (code, _stdout, stderr) = p.check_env(&script, lsp_dir, &[("PTAH_ASK", "stdin")]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("prompt"),
        "diagnostic must name the required field, stderr:\n{stderr}"
    );

    // type-definitions "Definitions model the sandbox" (os mirror):
    // `os.getenv` is a member of the trimmed os table — editor-approved
    // code using it analyzes clean.
    let p = Project::new("os-getenv-member");
    let script = p.write(
        "--!strict\n\
         local v: string? = os.getenv(\"HOME\")\n\
         print(v)\n",
    );
    let (code, stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        !stderr.contains("TypeError"),
        "os.getenv is a defs member — no diagnostic expected, stderr:\n{stderr}"
    );

    // Same requirement, rejecting side: `os.date` is trimmed from the
    // defs, so analysis flags it instead of the call failing at runtime.
    let p = Project::new("os-date-removed");
    let script = p.write("--!strict\n print(os.date(\"%Y\"))\n");
    let (code, _stdout, stderr) = p.check(&script, lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("Key 'date' not found in table"),
        "diagnostic must flag the trimmed os table, stderr:\n{stderr}"
    );
    // The rendered shape is the trimmed os surface itself — time,
    // clock, getenv — so the assertion also pins the mirror content.
    assert!(
        stderr.contains("getenv"),
        "diagnostic shows the trimmed os members, stderr:\n{stderr}"
    );
}
