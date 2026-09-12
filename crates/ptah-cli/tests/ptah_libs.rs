//! The pinned Ptah Playbooks library (`ptah-libs`) runs green against the
//! mock agent: every stdlib module and playbook entry point is exercised
//! offline. Each test builds a generated consumer project that installs the
//! library as the `ptah_libs` package from `PTAH_LIBS_SRC` (the flake-pinned
//! checkout the dev shell and the nix derivations export) — the real
//! consumption model — so the tests double as its regression suite. No
//! network, no real agent.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

/// The pinned library checkout: the `ptah-libs` flake input, exported as
/// `PTAH_LIBS_SRC` by the dev shell and the nix derivations. A hard failure
/// when absent — the suite cannot run without the library, and a silent skip
/// would hide exactly the regression it exists to catch.
fn library_src() -> PathBuf {
    std::env::var_os("PTAH_LIBS_SRC").map(PathBuf::from).expect(
        "PTAH_LIBS_SRC is not set: run the suite inside `nix develop` \
         (the dev shell exports the pinned ptah-libs checkout)",
    )
}

/// A minimal valid manifest for a generated project (the package name
/// segment is capped at pesde's 32-character limit).
fn manifest_skeleton(name: &str) -> String {
    let segment: String = name.replace('-', "_").chars().take(32).collect();
    format!(
        "name = \"components/{segment}\"\nversion = \"0.1.0\"\nprivate = true\n\n[target]\nenvironment = \"luau\"\n"
    )
}

/// A generated consumer project: a temp dir with a `.ptah/config.toml`
/// mapping the mock agent under several names (per-role env scripting) and
/// the pinned library installed as the `ptah_libs` package (path source —
/// fully offline). Shims require the package through the root `.luaurc`
/// alias, exactly as a real consumer would.
struct Project {
    dir: PathBuf,
}

impl Project {
    /// Judge-agent env scripting (see `new_env` for the general form).
    fn new(name: &str, judge_env: &[(&str, &str)]) -> Self {
        Self::new_env(name, "judge", judge_env)
    }

    /// `env_agent` names the registry entry (demo/judge/pi) the env
    /// knobs attach to — all names map to the mock agent; per-agent env
    /// is the per-role scripting surface.
    fn new_env(name: &str, env_agent: &str, env: &[(&str, &str)]) -> Self {
        Self::new_agents(name, &[(env_agent, env)])
    }

    /// Several agents carrying env knobs at once (e.g. the work agent's
    /// config echo and the judge agent's submit rules in one scenario).
    fn new_agents(name: &str, envs: &[(&str, &[(&str, &str)])]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ptah-factory-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ptah")).unwrap();
        // `demo` is the plain work agent, `judge` the typed-verdict
        // judge, `pi` the dogfood-shim agent — all the mock agent; the
        // env knobs attach to whichever names the scenario names.
        let env_for = |agent: &str| -> Vec<(&str, &str)> {
            envs.iter()
                .filter(|(a, _)| *a == agent)
                .flat_map(|(_, e)| e.iter().copied())
                .collect()
        };
        let mut config = String::new();
        let write_agent = |config: &mut String, name: &str| {
            config.push_str(&format!(
                "\n[agents.{name}]\ncommand = \"{}\"\nargs = []\n",
                mock_bin()
            ));
            let env = env_for(name);
            if !env.is_empty() {
                config.push_str(&format!("\n[agents.{name}.env]\n"));
                for (k, v) in env {
                    // TOML literal string: env values may carry double quotes (JSON).
                    config.push_str(&format!("{k} = '{v}'\n"));
                }
            }
        };
        write_agent(&mut config, "demo");
        write_agent(&mut config, "judge");
        write_agent(&mut config, "pi");
        std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();

        // Install the pinned library as the `ptah_libs` package: a path
        // source is fully offline (no registry, no git) and exercises the
        // same package machinery a consumer uses. The root `.luaurc` and
        // `.ptah/luau_packages/` appear as the install's output.
        std::fs::write(dir.join(".ptah/pesde.toml"), manifest_skeleton(name)).unwrap();
        let output = Command::new(ptah_bin())
            .args(["package", "add", "--path"])
            .arg(library_src())
            .args(["--as", "ptah_libs"])
            .current_dir(&dir)
            .env("HOME", &dir)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("run ptah package add");
        assert!(
            output.status.success(),
            "installing ptah_libs failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

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

    /// Run `ptah run <script>` from the project dir (HOME pinned so a
    /// developer's user registry cannot leak agents in).
    fn run(&self, script: &Path, extra: &[&str]) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("run")
            .args(extra)
            .arg(script)
            .current_dir(&self.dir)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("run ptah");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Like `run`, with `prepend` put in front of the child's PATH (for
    /// stub command fixtures such as `gh`).
    fn run_with_path(
        &self,
        script: &Path,
        prepend: &Path,
        extra: &[&str],
    ) -> (i32, String, String) {
        let path = std::env::var_os("PATH").expect("PATH is set");
        let joined = std::env::join_paths(
            std::iter::once(prepend.to_path_buf()).chain(std::env::split_paths(&path)),
        )
        .expect("join PATH");
        let output = Command::new(ptah_bin())
            .arg("run")
            .args(extra)
            .arg(script)
            .current_dir(&self.dir)
            .env("PATH", joined)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("run ptah");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Run `ptah check <script>` with `path` as the child's entire PATH
    /// (the real luau-lsp's directory — same discovery rule as
    /// tests/check.rs and tests/analyze.rs).
    fn check(&self, script: &Path, path: &Path) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("check")
            .arg("--no-color")
            .arg(script)
            .current_dir(&self.dir)
            .env("PATH", path)
            .env("HOME", &self.dir)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("run ptah check");
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

// ---------------------------------------------------------------------
// std/predicate — the typed judge
// ---------------------------------------------------------------------

/// Judge verdicts scripted through MOCK_SUBMIT_MATCH: every rule set
/// used by these tests, keyed on prompt substrings.
fn always(verdict: bool) -> String {
    format!(r#"[{{"match":"","value":{verdict}}}]"#)
}

#[test]
fn predicate_returns_the_submitted_verdict() {
    let p = Project::new("predicate-verdict", &[("MOCK_SUBMIT_MATCH", &always(true))]);
    let script = p.write(
        "main.luau",
        "--!strict\n\
         local predicate = require(\"@ptah_libs\").std.predicate\n\
         local verdict = predicate(\n\
         \t\"The payload mentions ptah\",\n\
         \t\"ptah drives agents\",\n\
         \t{ agent = ptah.agent(\"judge\"), sessionId = \"judge-1\", sessionConfig = { { id = \"model\", value = \"flash\" } } }\n\
         )\n\
         print(\"verdict=\" .. tostring(verdict))\n",
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("verdict=true"), "stdout: {stdout}");
}

#[test]
fn predicate_no_verdict_is_a_bounded_script_error() {
    // Rules that never match: the judge session submits nothing on every
    // attempt. The retry bound turns the would-be hang into a script
    // error naming the judge and the attempt count.
    let never = r#"[{"match":"no-such-substring","value":true}]"#;
    let p = Project::new("predicate-no-verdict", &[("MOCK_SUBMIT_MATCH", never)]);
    let script = p.write(
        "main.luau",
        "--!strict\n\
         local predicate = require(\"@ptah_libs\").std.predicate\n\
         predicate(\"p\", \"payload\", { agent = ptah.agent(\"judge\"), sessionId = \"judge-x\", maxAttempts = 3 })\n",
    );
    let (code, _stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("judge-x") && stderr.contains("3 attempts"),
        "error must name the judge and the attempt count, stderr:\n{stderr}"
    );
}

#[test]
fn predicate_session_config_reaches_the_judge_session_in_declared_order() {
    // The judge session is component-internal, so observability rides
    // the verdict channel: the mock gates an accepting rule on the
    // judge session's live effort value (requiresConfig), which — under
    // the re-derive contract — is `high` at prompt time only when the
    // entries were applied in declared order on the freshly created
    // session. Three calls in one judge process (each session re-seeds
    // at creation): declared order accepts, reversed order rejects,
    // no entries reject.
    let rules = r#"[{"match":"","value":true,"requiresConfig":{"effort":"high"}},{"match":"","value":false}]"#;
    let p = Project::new(
        "predicate-config-order",
        &[
            ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
            ("MOCK_CONFIG_DEPENDENT", "1"),
            ("MOCK_SUBMIT_MATCH", rules),
        ],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local predicate = require("@ptah_libs").std.predicate
local judge = ptah.agent("judge")
local inOrder = predicate("p", "payload", {
	agent = judge,
	sessionId = "j-ordered",
	maxAttempts = 1,
	sessionConfig = { { id = "model", value = "haiku" }, { id = "effort", value = "high" } },
})
local reversed = predicate("p", "payload", {
	agent = judge,
	sessionId = "j-reversed",
	maxAttempts = 1,
	sessionConfig = { { id = "effort", value = "high" }, { id = "model", value = "haiku" } },
})
local none = predicate("p", "payload", {
	agent = judge,
	sessionId = "j-none",
	maxAttempts = 1,
})
print(("verdicts=%s,%s,%s"):format(tostring(inOrder), tostring(reversed), tostring(none)))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("verdicts=true,false,false"),
        "entries must reach the judge session in declared order before the prompt, stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------
// std/session-config — the shared apply-in-order mechanism
// ---------------------------------------------------------------------

/// Select `model` + dependent select `effort` options the mock
/// advertises (defaults `opus`/`low`); with `MOCK_CONFIG_DEPENDENT` a
/// `model` set re-derives `effort` to its seeded default, modeling
/// agents with dependent options — the order-sensitive contract the
/// declared-order assertions lean on.
const CONFIG_OPTIONS_JSON: &str = r#"[{"id":"model","name":"Model","type":"select","currentValue":"opus","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]},{"id":"effort","name":"Effort","type":"select","currentValue":"low","options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}]"#;

#[test]
fn session_config_entries_apply_in_declared_order() {
    // The array is the setConfig call sequence as data: model first,
    // effort after — under the mock's re-derive contract only that
    // order lets the effort value stick. The same entries in the
    // reverse order end with effort re-derived to its default, which is
    // the order-sensitivity proof.
    let p = Project::new_env(
        "session-config-order",
        "demo",
        &[
            ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
            ("MOCK_CONFIG_DEPENDENT", "1"),
        ],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local sessionConfig = require("@ptah_libs").std.sessionConfig
local agent = ptah.agent("demo")

local ordered = agent:session({ id = "ordered" })
sessionConfig.apply(ordered, { { id = "model", value = "haiku" }, { id = "effort", value = "high" } })
local o = ordered:configOptions()
print("ordered model=" .. o[1].currentValue .. " effort=" .. o[2].currentValue)
ordered:close()

local reversed = agent:session({ id = "reversed" })
sessionConfig.apply(reversed, { { id = "effort", value = "high" }, { id = "model", value = "haiku" } })
local r = reversed:configOptions()
print("reversed model=" .. r[1].currentValue .. " effort=" .. r[2].currentValue)
reversed:close()
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("ordered model=haiku effort=high"),
        "model-then-effort must let the effort set stick, stdout: {stdout}"
    );
    assert!(
        stdout.contains("reversed model=haiku effort=low"),
        "effort-then-model must re-derive effort to its default (order is load-bearing), stdout: {stdout}"
    );
}

#[test]
fn session_config_nil_or_empty_entries_are_a_noop() {
    let p = Project::new_env(
        "session-config-noop",
        "demo",
        &[("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON)],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local sessionConfig = require("@ptah_libs").std.sessionConfig
local s = ptah.agent("demo"):session({ id = "noop" })
sessionConfig.apply(s, nil)
sessionConfig.apply(s, {})
local o = s:configOptions()
print("noop model=" .. o[1].currentValue .. " effort=" .. o[2].currentValue)
s:close()
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("noop model=opus effort=low"),
        "nil and empty must issue no setConfig calls (seeded defaults untouched), stdout: {stdout}"
    );
}

#[test]
fn session_config_duplicate_ids_apply_verbatim_last_wins() {
    // Repeated ids are applied verbatim in order — the library does not
    // invent a stricter contract than repeated runtime setConfig calls.
    let p = Project::new_env(
        "session-config-dupes",
        "demo",
        &[
            ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
            ("MOCK_CONFIG_DEPENDENT", "1"),
        ],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local sessionConfig = require("@ptah_libs").std.sessionConfig
local s = ptah.agent("demo"):session({ id = "dupes" })
sessionConfig.apply(s, { { id = "model", value = "haiku" }, { id = "model", value = "opus" } })
local o = s:configOptions()
print("dupes model=" .. o[1].currentValue .. " effort=" .. o[2].currentValue)
s:close()
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("dupes model=opus effort=low"),
        "the later duplicate must win, stdout: {stdout}"
    );
}

#[test]
fn session_config_agent_rejection_raises_the_set_config_error() {
    // MOCK_CONFIG_REJECT makes the agent fail the effort entry: apply
    // raises the existing setConfig error (catchable, carrying the
    // option id and the agent's message), and the model entry applied
    // before the rejection remains applied.
    let p = Project::new_env(
        "session-config-reject",
        "demo",
        &[
            ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
            ("MOCK_CONFIG_REJECT", "effort"),
        ],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local sessionConfig = require("@ptah_libs").std.sessionConfig
local s = ptah.agent("demo"):session({ id = "rej" })
-- The wrapper's return type gives pcall a second value to type-check
-- against (apply returns nothing; the runtime error object is the
-- second pcall return).
local ok, err = pcall(function(): string?
	sessionConfig.apply(s, { { id = "model", value = "haiku" }, { id = "effort", value = "high" } })
	return nil
end)
print("applied=" .. tostring(ok))
print("error=" .. tostring(err))
local o = s:configOptions()
print("model-after=" .. o[1].currentValue)
s:close()
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("applied=false"), "stdout: {stdout}");
    assert!(
        stdout.contains("setConfig(\"effort\") failed")
            && stdout.contains("mock-agent rejects config id effort"),
        "the error must carry the option id and the agent's message, stdout: {stdout}"
    );
    assert!(
        stdout.contains("model-after=haiku"),
        "entries applied before the rejection must remain applied, stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------
// std/gh — the GitHub CLI transport
// ---------------------------------------------------------------------

/// A stub `gh` executable: `echo-args` prints its arguments verbatim
/// (one per line — the quoting proof), `fail` exits non-zero with
/// stderr, anything else succeeds with whitespace-decorated JSON on
/// stdout (the trim proof).
fn stub_gh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ptah-ghstub-{}-{name}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let stub = dir.join("gh");
    std::fs::write(
        &stub,
        r#"#!/bin/sh
# echo-args: one argument per line, verbatim (quoting proof)
if [ "$1" = echo-args ]; then
  printf '%s\n' "$@"
  exit 0
fi
if [ "$1" = fail ]; then
  echo 'gh: Not Found (HTTP 404)' >&2
  exit 4
fi
case " $* " in
  *" --fail "*)
    echo 'gh: Not Found (HTTP 404)' >&2
    exit 4
    ;;
esac
printf '  {"number": 6, "title": "ok"}  '
exit 0
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

#[test]
fn gh_success_returns_parsed_json_and_trims_output() {
    let p = Project::new("gh-success", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local gh = require("@ptah_libs").std.gh
local o = gh.run({ "pr", "view", "6" }, { json = true })
print(("ok=%s exit=%s n=%s title=%s"):format(tostring(o.ok), tostring(o.exitCode), tostring(o.json.number), tostring(o.json.title)))
print("stdout=[" .. o.stdout .. "]")
"#,
    );
    let (code, stdout, stderr) = p.run_with_path(&script, &stub_gh("success"), &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("ok=true exit=0 n=6 title=ok"),
        "stdout: {stdout}"
    );
    // Whitespace the stub decorated its output with is stripped at both
    // ends (two-sided trim).
    assert!(
        stdout.contains("stdout=[{\"number\": 6, \"title\": \"ok\"}]"),
        "stdout: {stdout}"
    );
}

#[test]
fn gh_failure_is_data_not_an_error() {
    let p = Project::new("gh-failure", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local gh = require("@ptah_libs").std.gh
local o = gh.run({ "pr", "view", "999", "--fail" }, { json = true })
print(("ok=%s exit=%s json=%s"):format(tostring(o.ok), tostring(o.exitCode), tostring(o.json)))
print("stderr:" .. o.stderr)
"#,
    );
    let (code, stdout, stderr) = p.run_with_path(&script, &stub_gh("failure"), &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("ok=false exit=4 json=nil"),
        "the failed command must be a returned outcome, stdout: {stdout}"
    );
    assert!(stdout.contains("stderr:gh: Not Found"), "stdout: {stdout}");
}

#[test]
fn gh_quotes_arguments_verbatim() {
    let p = Project::new("gh-quoting", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local gh = require("@ptah_libs").std.gh
local o = gh.run({ "echo-args", "two words", "it's quoted", "a'b'c" })
print(o.stdout)
"#,
    );
    let (code, stdout, stderr) = p.run_with_path(&script, &stub_gh("quoting"), &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // Each argument arrives as one argv entry with spaces and embedded
    // single quotes intact.
    for expected in ["two words", "it's quoted", "a'b'c"] {
        assert!(stdout.contains(expected), "stdout: {stdout}");
    }
}

// ---------------------------------------------------------------------
// std/daemon — the per-repo loop skeleton
// ---------------------------------------------------------------------

/// One repo raising must not abort the others: `b` fails, `a` and `c`
/// complete, the run exits 0, and every repo has an outcome entry.
fn daemon_shim(p: &Project, concurrency: Option<u8>) -> PathBuf {
    let parallel = match concurrency {
        Some(n) => format!("{{ concurrency = {n} }}"),
        None => "nil".to_string(),
    };
    p.write(
        "main.luau",
        &format!(
            r#"--!strict
local daemon = require("@ptah_libs").std.daemon
local outcomes = daemon.each({{ "a", "b", "c" }}, function(repo: string)
	if repo == "b" then
		error("repo b is on fire")
	end
	print("done:" .. repo)
end, {parallel})
for _, o in ipairs(outcomes) do
	print(("%s=%s"):format(o.repo, tostring(o.ok)))
end
"#
        ),
    )
}

#[test]
fn daemon_sequential_survives_one_raising_repo() {
    let p = Project::new("daemon-sequential", &[]);
    let script = daemon_shim(&p, None);
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("done:a") && stdout.contains("done:c"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("a=true") && stdout.contains("b=false") && stdout.contains("c=true"),
        "every repo must have an outcome, stdout: {stdout}"
    );
}

#[test]
fn daemon_parallel_survives_one_raising_repo() {
    let p = Project::new("daemon-parallel", &[]);
    let script = daemon_shim(&p, Some(2));
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("done:a") && stdout.contains("done:c"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("a=true") && stdout.contains("b=false") && stdout.contains("c=true"),
        "every repo must have an outcome, stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------
// components/openspec — groom, implement, verify
// ---------------------------------------------------------------------

/// Judge rules for a component loop run: the second pass is accepted
/// (prompts carry the `[<id> iteration N of M]` header and the mock
/// echoes prompts back into the judge payload), the escalation
/// predicate (its text is embedded in the judge prompt) never needs a
/// human, and everything else fails. Shared by the openspec,
/// pr-review-loop, and dogfood tests below.
fn converges_on_second_pass() -> String {
    r#"[{"match":"iteration 2","value":true},{"match":"Human input is required","value":false},{"match":"","value":false}]"#.to_string()
}

fn openspec_shim(p: &Project, op: &str) -> PathBuf {
    p.write(
        "main.luau",
        &format!(
            r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({{
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = {{ {{ id = "model", value = "work-model" }} }},
	judgeSessionConfig = {{ {{ id = "model", value = "judge-model" }} }},
}})
local text = ops:{op}("demo-change")
print("{op}-ok:" .. tostring(text ~= nil))
"#
        ),
    )
}

#[test]
fn openspec_component_grooms_a_change() {
    // Review fails on the first pass (findings judged fixable), the fix
    // lands, the second pass converges — the full groom loop.
    let p = Project::new(
        "openspec-groom",
        &[("MOCK_SUBMIT_MATCH", &converges_on_second_pass())],
    );
    let script = openspec_shim(&p, "groom");
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("groom-ok:true"), "stdout: {stdout}");
}

#[test]
fn openspec_component_implements_a_change() {
    // Nil scope (the two-argument call): byte-for-byte compatibility —
    // today's unscoped work prompt and judge predicate still reach the
    // agents (the mock echoes prompts back, so both surfaces are
    // assertable), and no scope marker leaks into the run.
    let p = Project::new(
        "openspec-implement",
        &[("MOCK_SUBMIT_MATCH", &always(true))],
    );
    let script = openspec_shim(&p, "implement");
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("implement-ok:true"), "stdout: {stdout}");
    assert!(
        stdout.contains(
            "End each pass either with all tasks implemented or paused with a stated reason, as the skill defines those states."
        ),
        "the unscoped work prompt must be unchanged, stdout: {stdout}"
    );
    assert!(
        stdout.contains("All tasks of the change are implemented"),
        "the unscoped accepted predicate must be unchanged, stdout: {stdout}"
    );
    assert!(
        !stdout.contains("task scope"),
        "a nil-scope run must carry no scope markers, stdout: {stdout}"
    );
}

#[test]
fn openspec_component_implements_a_scoped_change() {
    // A task scope narrows the run to the scoped tasks: the scope text
    // must reach exactly the two interpolation sites — the work prompt
    // (echoed back by the mock) and the judge's accepted predicate
    // (embedded in the judge prompt, which the mock echoes too) — since
    // the judge never sees the work prompt. The scoped log line pins
    // the third scoped surface.
    let p = Project::new(
        "openspec-implement-scoped",
        &[("MOCK_SUBMIT_MATCH", &always(true))],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = { { id = "model", value = "work-model" } },
	judgeSessionConfig = { { id = "model", value = "judge-model" } },
})
local text = ops:implement("demo-change", "task group 1")
print("scoped-implement-ok:" .. tostring(text ~= nil))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("scoped-implement-ok:true"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("The task scope for this run is: task group 1"),
        "the scope must reach the work prompt, stdout: {stdout}"
    );
    assert!(
        stdout.contains("Treat the tasks matching the scope as the entire job"),
        "the scoped work prompt must redefine the job and leave other tasks pending, stdout: {stdout}"
    );
    assert!(
        stdout.contains("All tasks in the following task scope are implemented: \"task group 1\""),
        "the scope must ride inside the accepted predicate the judge sees, stdout: {stdout}"
    );
    assert!(
        stdout.contains("openspec: implementing change demo-change (task scope: task group 1)"),
        "the scoped log line must name the scope, stdout: {stdout}"
    );
}

#[test]
fn openspec_component_implements_an_unresolvable_scope_fails() {
    // A scope that matches no tasks dead-ends through the existing
    // escalation path: the work prompt's "state it, don't guess"
    // clause reaches the agent, the judge rejects the scoped accepted
    // predicate, the human probe confirms human input — and the
    // operation fails (exit 1) without ever issuing the resolve
    // prompt.
    let rules =
        r#"[{"match":"The pause requires human input","value":true},{"match":"","value":false}]"#
            .to_string();
    let p = Project::new(
        "openspec-implement-dead-end",
        &[("MOCK_SUBMIT_MATCH", &rules)],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = { { id = "model", value = "work-model" } },
	judgeSessionConfig = { { id = "model", value = "judge-model" } },
})
local text = ops:implement("demo-change", "no such tasks")
print("dead-end-ok:" .. tostring(text ~= nil))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 1, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("human input is required"),
        "the dead-end must surface through the escalation error, stderr: {stderr}"
    );
    assert!(
        stdout.contains(
            "If the task scope matches no tasks, end the pass stating that; do not guess or substitute"
        ),
        "the dead-end clause must reach the work agent, stdout: {stdout}"
    );
    assert!(
        stdout.contains("All tasks in the following task scope are implemented: \"no such tasks\""),
        "the scoped predicate must reach the judge before the escalation, stdout: {stdout}"
    );
    assert!(
        !stdout.contains("Go ahead and resolve the pause yourself"),
        "escalation must fail before the resolve prompt reaches the agent, stdout:\n{stdout}"
    );
}

#[test]
fn openspec_component_verify_converges_then_archives() {
    // Verification passes on the first pass; the sync-and-archive step
    // must run in the same operation — the mock echoes prompts back, so
    // the archive prompt is visible in the rendered output.
    let p = Project::new("openspec-verify", &[("MOCK_SUBMIT_MATCH", &always(true))]);
    let script = openspec_shim(&p, "verify");
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("verify-ok:true"), "stdout: {stdout}");
    assert!(
        stdout.contains("Please sync and archive the change demo-change"),
        "the archive prompt must reach the agent after convergence, stdout: {stdout}"
    );
}

#[test]
fn openspec_component_escalation_fails_without_a_fix() {
    // Groom is judge-rejected and the escalation judge confirms human
    // input is required: the operation must fail (exit 1) naming the
    // human, without ever issuing the fix prompt.
    let rules = r#"[{"match":"Human input is required","value":true},{"match":"","value":false}]"#
        .to_string();
    let p = Project::new("openspec-escalation", &[("MOCK_SUBMIT_MATCH", &rules)]);
    let script = openspec_shim(&p, "groom");
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 1, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stderr.contains("human input is required"),
        "stderr: {stderr}"
    );
    assert!(
        !stdout.contains("Go ahead and resolve the findings"),
        "escalation must fail before the fix prompt reaches the agent, stdout:\n{stdout}"
    );
}

#[test]
fn openspec_component_iteration_cap_fails() {
    // Every pass is judge-rejected and the findings stay fixable: the
    // loop runs to the configured cap and reports it.
    let rules = r#"[{"match":"Human input is required","value":false},{"match":"","value":false}]"#
        .to_string();
    let p = Project::new("openspec-cap", &[("MOCK_SUBMIT_MATCH", &rules)]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	maxIterations = 2,
})
ops:groom("demo-change")
"#,
    );
    let (code, _stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("did not converge within 2 iterations"),
        "stderr: {stderr}"
    );
}

// --- session-config propagation (the mock re-seeds option state at
// --- every session/new, so a session's config echo only carries the
// --- entries the component applied to that very session) --------------

#[test]
fn openspec_work_and_archive_sessions_receive_session_config() {
    // The work agent echoes its live effort value in every reply
    // (MOCK_CONFIG_ECHO); under the re-derive contract that value is
    // `high` only where the component applied the entries in declared
    // order. verify touches both work surfaces — the per-iteration
    // session (whose accepted text is returned) and the archive
    // session — so any session missing its entries would leak the
    // seeded default (`low`).
    let p = Project::new_agents(
        "openspec-config-work-archive",
        &[
            (
                "demo",
                &[
                    ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
                    ("MOCK_CONFIG_DEPENDENT", "1"),
                    ("MOCK_CONFIG_ECHO", "effort"),
                ],
            ),
            (
                "judge",
                &[("MOCK_SUBMIT_MATCH", r#"[{"match":"","value":true}]"#)],
            ),
        ],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = { { id = "model", value = "haiku" }, { id = "effort", value = "high" } },
})
local text = ops:verify("demo-change")
print("verify-ok:" .. text)
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("verify-ok:high"),
        "the accepted work-session text must carry the applied effort value, stdout: {stdout}"
    );
    assert!(
        !stdout.contains("] low"),
        "no session may leak the seeded default — work and archive sessions both got the entries, stdout:\n{stdout}"
    );

    // The same entries in the reverse order re-derive effort to the
    // seeded default everywhere — the order is the component's, applied
    // verbatim per session.
    let p = Project::new_agents(
        "openspec-config-work-archive-reversed",
        &[
            (
                "demo",
                &[
                    ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
                    ("MOCK_CONFIG_DEPENDENT", "1"),
                    ("MOCK_CONFIG_ECHO", "effort"),
                ],
            ),
            (
                "judge",
                &[("MOCK_SUBMIT_MATCH", r#"[{"match":"","value":true}]"#)],
            ),
        ],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = { { id = "effort", value = "high" }, { id = "model", value = "haiku" } },
})
local text = ops:verify("demo-change")
print("verify-ok:" .. text)
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("verify-ok:low"),
        "reversed entries must re-derive effort on every work session, stdout: {stdout}"
    );
    assert!(
        !stdout.contains("] high"),
        "reversed entries must never leave effort=high, stdout:\n{stdout}"
    );
}

#[test]
fn openspec_judge_and_probe_sessions_receive_judge_session_config() {
    // Judge observability rides the verdict channel: the accepting rule
    // is gated on the judge session's live effort value, which (freshly
    // re-seeded at session creation) is `high` only when the forwarded
    // judgeSessionConfig was applied in declared order before the
    // predicate prompt.
    let rules = r#"[{"match":"Human input is required","value":true,"requiresConfig":{"effort":"high"}},{"match":"All tasks","value":true,"requiresConfig":{"effort":"high"}},{"match":"","value":false}]"#;
    let judge_env: Vec<(&str, &str)> = vec![
        ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
        ("MOCK_CONFIG_DEPENDENT", "1"),
        ("MOCK_SUBMIT_MATCH", rules),
    ];
    let shim = |session_config_line: &str, max_iters: &str| {
        format!(
            r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({{
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
{session_config_line}	maxIterations = {max_iters},
}})
local text = ops:implement("demo-change")
print("implement-ok:" .. tostring(text ~= nil))
"#
        )
    };

    // The judge session for the accepted predicate: gated accept only
    // fires when the entries reached it in declared order.
    let p = Project::new_agents("openspec-config-judge", &[("judge", &judge_env)]);
    let script = p.write(
        "main.luau",
        &shim("\tjudgeSessionConfig = { { id = \"model\", value = \"haiku\" }, { id = \"effort\", value = \"high\" } },\n", "2"),
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("implement-ok:true"),
        "the accepted-predicate judge session must receive the forwarded entries, stdout: {stdout}"
    );

    // The human-escalation-probe session: the groom run's accepted
    // predicate stays rejected, and the probe's judge gate (effort
    // == high) confirms human input — reachable only through a
    // configured probe session.
    let p = Project::new_agents("openspec-config-probe", &[("judge", &judge_env)]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
local ops = openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	judgeSessionConfig = { { id = "model", value = "haiku" }, { id = "effort", value = "high" } },
	maxIterations = 2,
})
ops:groom("demo-change")
"#,
    );
    let (code, _stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("human input is required"),
        "the probe session must receive the entries (gated escalation confirms), stderr: {stderr}"
    );

    // Negative: reversed entries never satisfy the gates — the loop
    // exhausts the cap instead of converging.
    let p = Project::new_agents("openspec-config-judge-reversed", &[("judge", &judge_env)]);
    let script = p.write(
        "main.luau",
        &shim("\tjudgeSessionConfig = { { id = \"effort\", value = \"high\" }, { id = \"model\", value = \"haiku\" } },\n", "2"),
    );
    let (code, _stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("did not converge within 2 iterations"),
        "reversed entries must not satisfy the judge gates, stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------
// playbooks/pr-review-loop — convergent review→fix→push loop
// ---------------------------------------------------------------------

/// A stub `gh` for the loop's ledger transport: an empty comment list (no
/// ledger yet — the discovery pass), create/update endpoints, and a fixed
/// PR head SHA. No network, no state.
fn stub_gh_pr(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ptah-ghpr-{}-{name}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let stub = dir.join("gh");
    std::fs::write(
        &stub,
        r#"#!/bin/sh
# The review loop's gh surface: comment list/create/update + PR head.
case "$1" in
  api)
    method=GET
    prev=""
    for arg in "$@"; do
      [ "$prev" = "--method" ] && method="$arg"
      prev="$arg"
    done
    if [ "$method" = "POST" ] || [ "$method" = "PATCH" ]; then
      printf '{"id": 1}'
    else
      printf '[]'
    fi
    ;;
  pr)
    case "$*" in
      *"title,body"*) printf '{"title":"Test PR","body":"Add a feature.","headRefOid":"cafe1234","commits":[]}' ;;
      *) printf '{"headRefOid":"cafe1234"}' ;;
    esac
    ;;
  *) printf '{}' ;;
esac
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// A judge ruleset filing one blocking finding on the first pass and
/// resolving it on the second — the fix-then-converge shape. The judge
/// prompt embeds the review prose (the work prompt echoed back), so the
/// iteration header distinguishes the passes.
fn pr_review_converges_after_fix() -> String {
    r#"[{"match":"iteration 2","value":{"findings":[],"priorFindingsStatus":[{"id":"f1","status":"resolved","evidence":"fixed in cafe1234"}]}},{"match":"","value":{"findings":[{"severity":"blocking","family":"bugs","validation":"validated","needsHuman":false,"status":"open","title":"off-by-one"}],"priorFindingsStatus":[]}}]"#.to_string()
}

/// The pr-review shim: `config` is the constructor's extra config lines
/// (already indented); the run prints the outcome status and verdict.
fn pr_review_shim(p: &Project, config: &str) -> PathBuf {
    p.write(
        "main.luau",
        &format!(
            r#"--!strict
local libs = require("@ptah_libs")
local loop = libs.prReviewLoop.new({{
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
{config}}})
local outcome = loop:review("https://github.com/example/example/pull/6")
print("status=" .. outcome.status)
print("verdict=" .. outcome.verdict)
"#
        ),
    )
}

#[test]
fn pr_review_loop_converges_after_a_fix_and_push() {
    // The judge files a blocking finding on the discovery pass and
    // resolves it on the second: the loop reviews, fixes, pushes, and
    // converges — the fix and push prompts must reach the agent, and the
    // converged session posts the verdict comment.
    let p = Project::new(
        "pr-review-converge",
        &[("MOCK_SUBMIT_MATCH", &pr_review_converges_after_fix())],
    );
    let script = pr_review_shim(&p, "");
    let (code, stdout, stderr) = p.run_with_path(&script, &stub_gh_pr("converge"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("status=converged"), "stdout: {stdout}");
    assert!(
        stdout.contains("Resolve all of the following blocking findings"),
        "the fix prompt must reach the agent, stdout: {stdout}"
    );
    assert!(
        stdout.contains("push them to the PR branch"),
        "the fix must be followed by the push prompt, stdout: {stdout}"
    );
    assert!(
        stdout.contains("Post a comment on pull request"),
        "the converged session must post the verdict comment, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_dry_run_never_pushes_but_still_comments() {
    // The dry-run gate skips the commit-and-push step, but the converged
    // session still posts the verdict comment (dry-run gates the branch,
    // not the PR conversation).
    let p = Project::new(
        "pr-review-dry-run",
        &[("MOCK_SUBMIT_MATCH", &pr_review_converges_after_fix())],
    );
    let script = pr_review_shim(&p, "\tdryRun = true,\n");
    let (code, stdout, stderr) = p.run_with_path(&script, &stub_gh_pr("dry-run"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("status=converged"), "stdout: {stdout}");
    assert!(
        !stdout.contains("push them to the PR branch"),
        "dry-run must never send the commit-and-push prompt, stdout: {stdout}"
    );
    assert!(
        stdout.contains("Post a comment on pull request"),
        "the converged session still posts the verdict comment in dry-run, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_configured_instruction_wins_over_default() {
    // A configured `reviewInstruction` fully replaces the built-in persona
    // (only nil selects the default), so the configured text is inlined
    // and the default's opening line never appears.
    let p = Project::new(
        "pr-review-instruction",
        &[("MOCK_SUBMIT_MATCH", &pr_review_converges_after_fix())],
    );
    let script = pr_review_shim(
        &p,
        "\treviewInstruction = \"Review for correctness first and label each finding blocking or non-blocking.\",\n",
    );
    let (code, stdout, stderr) =
        p.run_with_path(&script, &stub_gh_pr("instruction"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains(
            "Review for correctness first and label each finding blocking or non-blocking."
        ),
        "the configured instruction text must be inlined into the review prompt, stdout: {stdout}"
    );
    assert!(
        !stdout.contains("You are a code reviewer"),
        "the built-in default must not be inlined when an instruction is configured, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_work_sessions_receive_session_config() {
    // The work agent echoes its live effort value (MOCK_CONFIG_ECHO); the
    // judge converges immediately, so the returned verdict is that echo —
    // `high` only when the entries were applied in declared order on the
    // freshly created session.
    let demo_env: Vec<(&str, &str)> = vec![
        ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
        ("MOCK_CONFIG_DEPENDENT", "1"),
        ("MOCK_CONFIG_ECHO", "effort"),
    ];
    let judge_env: Vec<(&str, &str)> = vec![(
        "MOCK_SUBMIT_MATCH",
        r#"[{"match":"","value":{"findings":[],"priorFindingsStatus":[]}}]"#,
    )];

    let p = Project::new_agents(
        "pr-review-config-work",
        &[("demo", &demo_env), ("judge", &judge_env)],
    );
    let script = pr_review_shim(
        &p,
        "\tsessionConfig = { { id = \"model\", value = \"haiku\" }, { id = \"effort\", value = \"high\" } },\n",
    );
    let (code, stdout, stderr) =
        p.run_with_path(&script, &stub_gh_pr("config-work"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("verdict=high"),
        "the work session must receive the entries in declared order, stdout: {stdout}"
    );

    let p = Project::new_agents(
        "pr-review-config-work-reversed",
        &[("demo", &demo_env), ("judge", &judge_env)],
    );
    let script = pr_review_shim(
        &p,
        "\tsessionConfig = { { id = \"effort\", value = \"high\" }, { id = \"model\", value = \"haiku\" } },\n",
    );
    let (code, stdout, stderr) =
        p.run_with_path(&script, &stub_gh_pr("config-work-rev"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("verdict=low"),
        "reversed entries must re-derive effort on the work session, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_judge_sessions_receive_judge_session_config() {
    // The judge's accept is gated on the judge session's live effort
    // value: forwarded entries reach the freshly created session in
    // declared order before its prompt.
    let rules = r#"[{"match":"","value":{"findings":[],"priorFindingsStatus":[]},"requiresConfig":{"effort":"high"}},{"match":"","value":{"findings":[{"severity":"blocking","family":"bugs","validation":"validated","needsHuman":false,"status":"open","title":"bug"}],"priorFindingsStatus":[]}}]"#;
    let judge_env: Vec<(&str, &str)> = vec![
        ("MOCK_CONFIG_OPTIONS", CONFIG_OPTIONS_JSON),
        ("MOCK_CONFIG_DEPENDENT", "1"),
        ("MOCK_SUBMIT_MATCH", rules),
    ];

    let p = Project::new_agents("pr-review-config-judge", &[("judge", &judge_env)]);
    let script = pr_review_shim(
        &p,
        "\tjudgeSessionConfig = { { id = \"model\", value = \"haiku\" }, { id = \"effort\", value = \"high\" } },\n\tmaxIterations = 2,\n",
    );
    let (code, stdout, stderr) =
        p.run_with_path(&script, &stub_gh_pr("config-judge"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("status=converged"),
        "the gated accept must fire on a configured judge session, stdout: {stdout}"
    );

    let p = Project::new_agents("pr-review-config-judge-rev", &[("judge", &judge_env)]);
    let script = pr_review_shim(
        &p,
        "\tjudgeSessionConfig = { { id = \"effort\", value = \"high\" }, { id = \"model\", value = \"haiku\" } },\n\tmaxIterations = 2,\n",
    );
    let (code, stdout, stderr) =
        p.run_with_path(&script, &stub_gh_pr("config-judge-rev"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("status=non-converged"),
        "reversed entries must not satisfy the judge gate, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_needs_human_fails_without_a_provider() {
    // A judge finding flagged `needsHuman` routes through std/escalate; with
    // no ask provider the ask classifies `unavailable` and the loop fails
    // with the human-input error.
    let rules = r#"[{"match":"","value":{"findings":[{"severity":"blocking","family":"bugs","validation":"validated","needsHuman":true,"status":"open","title":"needs a decision"}],"priorFindingsStatus":[]}}]"#;
    let p = Project::new("pr-review-needs-human", &[("MOCK_SUBMIT_MATCH", rules)]);
    let script = pr_review_shim(&p, "");
    let (code, _stdout, stderr) =
        p.run_with_path(&script, &stub_gh_pr("needs-human"), &["--quiet"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("human input is required to resolve the findings"),
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------
// std/escalate — the ask transport (outcome as data)
// ---------------------------------------------------------------------

#[test]
fn escalate_unavailable_without_a_provider() {
    // No provider is configured and the run is not a TTY, so `ptah.ask`
    // raises and the transport classifies it as data — the script keeps
    // running.
    let p = Project::new("escalate-unavailable", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local escalate = require("@ptah_libs").std.escalate
local o = escalate.ask({ prompt = "Need a human", details = "context" })
print("status=" .. o.status)
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("status=unavailable"), "stdout: {stdout}");
}

#[test]
fn escalate_respond_returns_the_human_answer() {
    let p = Project::new("escalate-respond", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local escalate = require("@ptah_libs").std.escalate
local o = escalate.ask({ prompt = "Approve the change?", details = "full context" })
if o.status == "respond" then
	print("answer=" .. o.text)
else
	print("status=" .. o.status)
end
"#,
    );
    let mut run = common::PipedRun::spawn(&p.dir, &script, &["--ask=stdin"]);
    run.wait_for("Approve the change?");
    run.write_line("yes, ship it");
    run.wait_for("answer=yes, ship it");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");
}

#[test]
fn escalate_abort_is_data() {
    let p = Project::new("escalate-abort", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local escalate = require("@ptah_libs").std.escalate
local o = escalate.ask({ prompt = "Approve the change?" })
print("status=" .. o.status)
"#,
    );
    let mut run = common::PipedRun::spawn(&p.dir, &script, &["--ask=stdin"]);
    run.wait_for("Approve the change?");
    run.write_line("/abort");
    run.wait_for("status=abort");
    let (code, all, stderr) = run.finish();
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{all}");
}

// ---------------------------------------------------------------------
// The pin guard: the flake-pinned source is the tree the committed
// `.ptah/pesde.lock` pins (they must move together).
// ---------------------------------------------------------------------

/// Read `PTAH_LIBS_SRC` as `(relative path, contents)` pairs, skipping the
/// `.git` directory (a dev checkout has one; a nix flake input does not).
fn read_tree_files(root: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("read source dir") {
            let entry = entry.expect("dir entry");
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("path under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let contents = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                out.push((rel, contents));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// The `tree_id` recorded for the git dependency in the committed lockfile.
fn committed_lockfile_tree_id() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.ptah/pesde.lock");
    let text = std::fs::read_to_string(&path).expect("read .ptah/pesde.lock");
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("tree_id = ") {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    panic!("no git pkg_ref tree_id in .ptah/pesde.lock");
}

#[test]
fn flake_pin_matches_the_committed_lockfile_tree() {
    // Rebuild the pinned source as a git tree and compare its root tree id
    // to the lockfile's: a `ptah package update` must be paired with
    // `nix flake update ptah-libs`.
    let src = library_src();
    let files = read_tree_files(&src);
    let specs: Vec<ptah_pesde::fixtures::FileSpec<'_>> = files
        .iter()
        .map(|(path, contents)| ptah_pesde::fixtures::FileSpec {
            path: path.as_str(),
            contents: contents.as_str(),
        })
        .collect();
    let scratch = tempfile::tempdir().expect("tempdir");
    let fixture = ptah_pesde::fixtures::git_repo(scratch.path(), &specs);
    assert_eq!(
        fixture.tree_id.to_string(),
        committed_lockfile_tree_id(),
        "PTAH_LIBS_SRC is not the tree .ptah/pesde.lock pins: \
         pair `ptah package update` with `nix flake update ptah-libs`"
    );
}

// ---------------------------------------------------------------------
// Dogfooding: this repo's own .ptah/workflows/* shims (the consumer
// pattern, byte for byte) run against the mock agent.
// ---------------------------------------------------------------------

/// The repo's checked-in workflow shims (same tree the flake builds
/// from, so the sandbox runs these paths too).
fn workflow(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.ptah/workflows")
        .join(name)
}

#[test]
fn dogfood_openspec_shim_runs() {
    // The openspec shim as it exists in the repo: the groom/implement/
    // verify operations themselves are covered playbook-level above; this
    // pins that the actual shim runs against the mock. The shim is copied
    // into the generated project so its `@ptah_libs` require resolves
    // against that project's install (alias resolution is file-relative).
    let rules = r#"[{"match":"","value":true}]"#.to_string();
    let p = Project::new_env("dogfood-openspec", "pi", &[("MOCK_SUBMIT_MATCH", &rules)]);
    let shim = std::fs::read_to_string(workflow("openspec/main.luau")).unwrap();
    let script = p.write("openspec_shim.luau", &shim);
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("Please sync and archive the change"),
        "verify shim must run the archive step, stdout: {stdout}"
    );
}

#[test]
fn dogfood_pr_review_loop_shim_runs() {
    let p = Project::new_env(
        "dogfood-pr-review",
        "pi",
        &[("MOCK_SUBMIT_MATCH", &pr_review_converges_after_fix())],
    );
    let shim = std::fs::read_to_string(workflow("pr-review-loop/main.luau")).unwrap();
    let script = p.write("pr_review_shim.luau", &shim);
    let (code, stdout, stderr) = p.run_with_path(&script, &stub_gh_pr("dogfood"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("push them to the PR branch"),
        "review loop shim must push after the fix, stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------
// The compatibility gate: `ptah check` validates a consumer's config
// against the component's exported Config type (real analyzer).
// ---------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn mistyped_component_config_is_a_check_finding() {
    let Some(lsp) = luau_lsp_on_path() else {
        if std::env::var_os("PTAH_REQUIRE_REAL_LSP").is_some() {
            panic!("PTAH_REQUIRE_REAL_LSP is set but luau-lsp is not on PATH");
        }
        eprintln!("skipping: luau-lsp not on PATH (run inside `nix develop`)");
        return;
    };
    let lsp_dir = lsp
        .parent()
        .expect("luau-lsp path has a parent")
        .to_path_buf();

    // A typo'd key (missing the required `judgeAgent`): the diagnostic
    // must name the field. The valid field constructs its handle so the
    // typo is the isolated error.
    let p = Project::new("gate-typo", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
openspec.new({ agent = ptah.agent("demo"), judgeAgnt = ptah.agent("demo") })
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("judgeAgent"),
        "the diagnostic must name the missing config field, stderr:\n{stderr}"
    );

    // A wrong-typed field (`dryRun` as string): the diagnostic names the
    // field and the accepted type.
    let p = Project::new("gate-type", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local prReview = require("@ptah_libs").prReviewLoop
prReview.new({ agent = ptah.agent("demo"), judgeAgent = ptah.agent("judge"), reviewInstruction = "x.md", dryRun = "yes" })
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("dryRun") && stderr.contains("boolean"),
        "the diagnostic must name the field and its accepted type, stderr:\n{stderr}"
    );

    // A callable hook in a config field: the contract admits data and
    // declared runtime handles only, so an arbitrary function is a
    // type error naming the field.
    let p = Project::new("gate-hook", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
openspec.new({ agent = ptah.agent("demo"), judgeAgent = ptah.agent("judge"), maxIterations = function() return 3 end })
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("maxIterations"),
        "the diagnostic must name the callable-hook field, stderr:\n{stderr}"
    );

    // A shim configuring the removed `model`/`judgeModel` fields: the
    // components declare them nil-typed (the analyzer does not flag
    // unknown keys on its own), so a configured value is a type error
    // naming the field — steering the consumer to the sessionConfig
    // entry form.
    let p = Project::new("gate-removed-model", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	model = "opus",
	judgeModel = "haiku",
})
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("'model'") && stderr.contains("'judgeModel'"),
        "the diagnostic must name each removed field, stderr:\n{stderr}"
    );

    // A wrong-typed entry value: the diagnostic names the entry shape
    // (field and accepted union).
    let p = Project::new("gate-entry-value", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = { { id = "model", value = 42 } },
})
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("'value'") && stderr.contains("string"),
        "the diagnostic must name the entry field and its accepted type, stderr:\n{stderr}"
    );

    // A missing entry id: the diagnostic names the entry shape's
    // required field.
    let p = Project::new("gate-entry-id", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("@ptah_libs").openspec
openspec.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	sessionConfig = { { value = "x" } },
})
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("'id'") && stderr.contains("missing"),
        "the diagnostic must name the missing entry field, stderr:\n{stderr}"
    );

    // The gate's accepting side: a well-typed shim for every component
    // analyzes clean through the whole mounted require graph — handles
    // from registry names and, for one role, from an inline agent spec
    // (the registry-free consumer form) — and ordered session-config
    // entry arrays are accepted as data config (string and boolean
    // entry values alike).
    let p = Project::new("gate-clean", &[]);
    let script = p.write(
        "main.luau",
        format!(
            r#"--!strict
local openspec = require("@ptah_libs").openspec
local prReview = require("@ptah_libs").prReviewLoop
local inline = ptah.agent({{ command = "{mock}" }})
local ops = openspec.new({{
	agent = inline,
	judgeAgent = ptah.agent("judge"),
	maxIterations = 4,
	sessionConfig = {{ {{ id = "model", value = "opus" }} }},
	-- The analyzer infers one element type per unannotated literal, so
	-- the boolean value arm gets its own entry array:
	judgeSessionConfig = {{ {{ id = "stream", value = true }} }},
}})
local loop = prReview.new({{
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	reviewInstruction = "doc.md",
	dryRun = true,
	sessionConfig = {{ {{ id = "model", value = "opus" }} }},
}})
print(ops, loop)
"#,
            mock = mock_bin()
        )
        .as_str(),
    );
    let (code, stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        !stderr.contains("TypeError"),
        "well-typed shims must analyze clean, stderr:\n{stderr}"
    );
}
