//! The bundled Factory Components library (`factory-components/`) runs
//! green against the mock agent: every stdlib module and component entry
//! point is exercised offline — the library is mounted into a generated
//! project exactly like a consumer repo would mount it (copy at an
//! arbitrary path + thin shim), so the tests double as the consumption
//! model's regression suite. No network, no real agent.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

/// The library tree as it ships in this repo (the same tree the flake
/// keeps in the build source, so the sandbox runs these paths too).
fn library_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../factory-components")
}

/// A generated consumer project: a temp dir with a `.ptah/config.toml`
/// mapping the mock agent under several names (per-role env scripting),
/// and a copy of the library mounted at `vendor/factory-components` —
/// an arbitrary mount point, proving the tree is location-agnostic.
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
        // `judge` carries the env knobs (the typed-verdict scripting);
        // `demo` is the plain work agent; `pi` is the dogfood-shim agent.
        let mut config = format!(
            "[agents.demo]\ncommand = \"{}\"\nargs = []\n\n\
             [agents.judge]\ncommand = \"{}\"\nargs = []\n\n[agents.judge.env]\n",
            mock_bin(),
            mock_bin()
        );
        if env_agent == "judge" {
            for (k, v) in env {
                // TOML literal string: env values may carry double quotes (JSON).
                config.push_str(&format!("{k} = '{v}'\n"));
            }
        }
        config.push_str(&format!(
            "\n[agents.pi]\ncommand = \"{}\"\nargs = []\n",
            mock_bin()
        ));
        if env_agent == "pi" {
            config.push_str("\n[agents.pi.env]\n");
            for (k, v) in env {
                config.push_str(&format!("{k} = '{v}'\n"));
            }
        }
        std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();

        // Mount the library: a plain copy at an arbitrary path.
        let mounted = dir.join("vendor/factory-components");
        copy_tree(&library_src(), &mounted).unwrap();

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
    fn run_with_path(&self, script: &Path, prepend: &Path, extra: &[&str]) -> (i32, String, String) {
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

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
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
         local predicate = require(\"./vendor/factory-components/std/predicate\")\n\
         local verdict = predicate(\n\
         \t\"The payload mentions ptah\",\n\
         \t\"ptah drives agents\",\n\
         \t{ agent = ptah.agent(\"judge\"), sessionId = \"judge-1\", model = \"flash\" }\n\
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
         local predicate = require(\"./vendor/factory-components/std/predicate\")\n\
         predicate(\"p\", \"payload\", { agent = ptah.agent(\"judge\"), sessionId = \"judge-x\", maxAttempts = 3 })\n",
    );
    let (code, _stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("judge-x") && stderr.contains("3 attempts"),
        "error must name the judge and the attempt count, stderr:\n{stderr}"
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
local gh = require("./vendor/factory-components/std/gh")
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
local gh = require("./vendor/factory-components/std/gh")
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
local gh = require("./vendor/factory-components/std/gh")
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
local daemon = require("./vendor/factory-components/std/daemon")
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
    assert!(stdout.contains("done:a") && stdout.contains("done:c"), "stdout: {stdout}");
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
    assert!(stdout.contains("done:a") && stdout.contains("done:c"), "stdout: {stdout}");
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
local openspec = require("./vendor/factory-components/components/openspec/component")
local ops = openspec.new({{
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	model = "work-model",
	judgeModel = "judge-model",
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
    let p = Project::new("openspec-implement", &[("MOCK_SUBMIT_MATCH", &always(true))]);
    let script = openspec_shim(&p, "implement");
    let (code, stdout, stderr) = p.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("implement-ok:true"), "stdout: {stdout}");
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
    let rules =
        r#"[{"match":"Human input is required","value":true},{"match":"","value":false}]"#
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
    let rules =
        r#"[{"match":"Human input is required","value":false},{"match":"","value":false}]"#
            .to_string();
    let p = Project::new("openspec-cap", &[("MOCK_SUBMIT_MATCH", &rules)]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")
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

// ---------------------------------------------------------------------
// components/pr-review-loop — review→fix→push convergence
// ---------------------------------------------------------------------

#[test]
fn pr_review_loop_converges_review_fix_push() {
    // Default mode (no `reviewInstruction` — this repo's own
    // dogfood configuration): the loop runs against the built-in
    // default instruction. Judge rules: the second review pass passes
    // (the fix landed), the escalation predicate never needs a human,
    // everything else fails — so the loop runs review → fix → push and
    // converges. The push prompt must reach the agent (the mock echoes
    // prompts back), and the echoed review prompt must carry the
    // default's classification directive (uppercase BLOCKING — the
    // component's own ask is lowercase, so only the inlined default
    // text can match).
    let p = Project::new(
        "pr-review-loop",
        &[("MOCK_SUBMIT_MATCH", &converges_on_second_pass())],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local prReview = require("./vendor/factory-components/components/pr-review-loop/component")
local loop = prReview.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
})
local text = loop:review("https://github.com/example/example/pull/6")
print("review-ok:" .. tostring(text ~= nil))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("review-ok:true"), "stdout: {stdout}");
    assert!(
        stdout.contains("push them to the PR branch"),
        "the fix must be followed by the push prompt, stdout: {stdout}"
    );
    assert!(
        stdout.contains("BLOCKING"),
        "the built-in default instruction (classification directive) must reach the agent, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_dry_run_never_pushes_but_still_comments() {
    // Same judge rules as the push test (converge on the second pass),
    // with the dry-run gate on: the loop reviews, fixes, and converges,
    // but the commit-and-push prompt must never reach the agent — while
    // the converged session still posts the verdict comment (dry-run
    // gates the branch, not the PR conversation; see the README).
    let p = Project::new(
        "pr-review-dry-run",
        &[("MOCK_SUBMIT_MATCH", &converges_on_second_pass())],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local prReview = require("./vendor/factory-components/components/pr-review-loop/component")
local loop = prReview.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	dryRun = true,
})
local text = loop:review("https://github.com/example/example/pull/6")
print("review-ok:" .. tostring(text ~= nil))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("review-ok:true"), "stdout: {stdout}");
    assert!(
        !stdout.contains("push them to the PR branch"),
        "dry-run must never send the commit-and-push prompt, stdout: {stdout}"
    );
    assert!(
        stdout.contains("Please comment on the PR with the review feedback along with the verdict"),
        "the converged session still posts the verdict comment in dry-run, stdout: {stdout}"
    );
}

#[test]
fn pr_review_loop_configured_instruction_wins_over_default() {
    // Replace semantics: test-authored instruction text is
    // configured via `reviewInstruction`; the text must be inlined
    // into the echoed review prompt, while the built-in default's
    // classification directive (uppercase BLOCKING) must not appear —
    // a configured instruction fully replaces the default (the
    // test's own text deliberately avoids the uppercase directive,
    // so only the inlined default could match it).
    let p = Project::new(
        "pr-review-instruction-wins",
        &[("MOCK_SUBMIT_MATCH", &converges_on_second_pass())],
    );
    let script = p.write(
        "main.luau",
        r#"--!strict
local prReview = require("./vendor/factory-components/components/pr-review-loop/component")
local loop = prReview.new({
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	reviewInstruction = "Review for correctness first. Judge each finding against this repository's severity ladder and label it blocking or non-blocking.",
})
local text = loop:review("https://github.com/example/example/pull/6")
print("review-ok:" .. tostring(text ~= nil))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("review-ok:true"), "stdout: {stdout}");
    assert!(
        stdout.contains(
            "Review for correctness first. Judge each finding against this repository's severity ladder and label it blocking or non-blocking."
        ),
        "the configured instruction text must be inlined into the review prompt, stdout: {stdout}"
    );
    assert!(
        !stdout.contains("BLOCKING"),
        "the built-in default must not be inlined when an instruction is configured, stdout: {stdout}"
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
    // The consolidated openspec shim (groom/verify merged in
    // 7a63d32): the groom/implement/verify operations themselves are
    // covered component-level above; this pins that the repo's actual
    // shim runs against the mock — including the archive step of its
    // verify call and the typed PR-url handoff into the review loop:
    // the mock submits a schema-valid prUrl object for the PR-url
    // prompt (first matching rule) and true everywhere else, so every
    // judge predicate converges.
    let rules = r#"[{"match":"PR url","value":{"prUrl":"https://github.com/patextreme/ptah/pull/10"}},{"match":"","value":true}]"#
        .to_string();
    let p = Project::new_env("dogfood-openspec", "pi", &[("MOCK_SUBMIT_MATCH", &rules)]);
    let (code, stdout, stderr) = p.run(&workflow("openspec/main.luau"), &["--no-color"]);
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
        &[("MOCK_SUBMIT_MATCH", &converges_on_second_pass())],
    );
    let (code, stdout, stderr) = p.run(&workflow("pr-review-loop/main.luau"), &["--no-color"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("push them to the PR branch"),
        "review loop shim must push after the fix, stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------
// Read-only mount: the library tree works from a read-only location
// (e.g. the nix store) — no writes inside the tree, no relative-cwd
// dependence.
// ---------------------------------------------------------------------

#[cfg(unix)]
fn set_tree_mode(root: &Path, dir_mode: u32, file_mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(root).unwrap();
    std::fs::set_permissions(
        root,
        std::fs::Permissions::from_mode(if meta.is_dir() {
            dir_mode
        } else {
            file_mode
        }),
    )
    .unwrap();
    if meta.is_dir() {
        for entry in std::fs::read_dir(root).unwrap() {
            set_tree_mode(&entry.unwrap().path(), dir_mode, file_mode);
        }
    }
}

#[test]
#[cfg(unix)]
fn component_runs_from_a_read_only_mount() {
    let p = Project::new("read-only-mount", &[("MOCK_SUBMIT_MATCH", &always(true))]);
    // Re-mount the library read-only (dirs 0555, files 0444): any write
    // inside the tree would fail with EROFS and the run would error.
    let mounted = p.dir.join("vendor/factory-components");
    set_tree_mode(&mounted, 0o555, 0o444);
    // The shim lives elsewhere and the run is invoked from the project
    // dir — not the library's — pinning the no-relative-cwd contract.
    let script = p.write(
        "shim.luau",
        r#"--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")
local ops = openspec.new({ agent = ptah.agent("demo"), judgeAgent = ptah.agent("judge") })
local text = ops:verify("some-change")
print("readonly-mount-ok:" .. tostring(text ~= nil))
"#,
    );
    let (code, stdout, stderr) = p.run(&script, &["--no-color"]);
    // Restore writable modes so temp-dir cleanup can remove the tree.
    set_tree_mode(&mounted, 0o755, 0o644);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("readonly-mount-ok:true"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Please sync and archive the change some-change"),
        "the full operation must complete from the read-only mount, stdout: {stdout}"
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
    let lsp_dir = lsp.parent().expect("luau-lsp path has a parent").to_path_buf();

    // A typo'd key (missing the required `judgeAgent`): the diagnostic
    // must name the field. The valid field constructs its handle so the
    // typo is the isolated error.
    let p = Project::new("gate-typo", &[]);
    let script = p.write(
        "main.luau",
        r#"--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")
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
local prReview = require("./vendor/factory-components/components/pr-review-loop/component")
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
local openspec = require("./vendor/factory-components/components/openspec/component")
openspec.new({ agent = ptah.agent("demo"), judgeAgent = ptah.agent("judge"), maxIterations = function() return 3 end })
"#,
    );
    let (code, _stdout, stderr) = p.check(&script, &lsp_dir);
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("maxIterations"),
        "the diagnostic must name the callable-hook field, stderr:\n{stderr}"
    );

    // The gate's accepting side: a well-typed shim for every component
    // analyzes clean through the whole mounted require graph — handles
    // from registry names and, for one role, from an inline agent spec
    // (the registry-free consumer form).
    let p = Project::new("gate-clean", &[]);
    let script = p.write(
        "main.luau",
        format!(
            r#"--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")
local prReview = require("./vendor/factory-components/components/pr-review-loop/component")
local inline = ptah.agent({{ command = "{mock}" }})
local ops = openspec.new({{ agent = inline, judgeAgent = ptah.agent("judge"), maxIterations = 4 }})
local loop = prReview.new({{
	agent = ptah.agent("demo"),
	judgeAgent = ptah.agent("judge"),
	reviewInstruction = "doc.md",
	dryRun = true,
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
