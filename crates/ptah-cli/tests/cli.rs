//! CLI + renderer integration tests: run the real `ptah` binary against
//! the mock agent via a project registry and assert on captured output.

use std::path::PathBuf;
use std::process::Command;

mod common;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str, agent_env: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("ptah-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ptah")).unwrap();

        let mut config = format!("[agents.mock]\ncommand = \"{}\"\nargs = []\n", mock_bin());
        if !agent_env.is_empty() {
            config.push_str("\n[agents.mock.env]\n");
            for (k, v) in agent_env {
                // TOML literal strings: env values carry JSON (double
                // quotes) without escaping.
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

    /// Rewrite the agent env (same format as `new`), for env values that
    /// depend on the project's own directory (peek path fixtures).
    fn set_env(&self, agent_env: &[(&str, &str)]) {
        let mut config = format!("[agents.mock]\ncommand = \"{}\"\nargs = []\n", mock_bin());
        if !agent_env.is_empty() {
            config.push_str("\n[agents.mock.env]\n");
            for (k, v) in agent_env {
                config.push_str(&format!("{k} = '{v}'\n"));
            }
        }
        std::fs::write(self.dir.join(".ptah").join("config.toml"), config).unwrap();
    }

    fn run(&self, script: &PathBuf, flags: &[&str]) -> (i32, String, String) {
        let output = Command::new(ptah_bin())
            .arg("run")
            .arg(script)
            .args(flags)
            .current_dir(&self.dir)
            .env_remove("PTAH_TEST")
            .output()
            .expect("run ptah");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

#[test]
fn version_flag() {
    let out = Command::new(ptah_bin()).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.trim().is_empty(), "{stdout}");
}

#[test]
fn types_prints_version_header_and_definitions() {
    // `ptah types` must emit a one-line version header followed by the
    // repo definitions byte-for-byte (spec: suitable for redirection).
    let out = Command::new(ptah_bin()).arg("types").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (header, body) = stdout
        .split_once('\n')
        .unwrap_or_else(|| panic!("no header line: {stdout:?}"));
    assert_eq!(
        header,
        format!("-- ptah {} type definitions", env!("CARGO_PKG_VERSION"))
    );
    let repo_defs = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.ptah/ptah.d.luau"),
    )
    .unwrap();
    assert_eq!(body, repo_defs, "emitted defs must be byte-identical");
}

#[test]
fn types_needs_no_registry_or_agents() {
    // No script, no registry (empty HOME and cwd), no agent spawned: still
    // succeeds. A spawned agent would need a registry entry to come from.
    let dir = std::env::temp_dir().join(format!("ptah-cli-types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(ptah_bin())
        .arg("types")
        .current_dir(&dir)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "expected no diagnostics: {stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("declare ptah"), "{stdout}");
}

#[test]
fn missing_script_argument_is_usage_error() {
    let out = Command::new(ptah_bin()).arg("run").output().unwrap();
    assert_ne!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.to_lowercase().contains("usage"), "{stderr}");
}

#[test]
fn nonexistent_script_names_path() {
    let out = Command::new(ptah_bin())
        .arg("run")
        .arg("/definitely/not/here.luau")
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("/definitely/not/here.luau"), "{stderr}");
}

#[test]
fn relative_script_path_with_require_runs() {
    // A script invoked through a relative path (with a directory
    // component) must still be able to require sibling modules: the
    // require sandbox root must share the absolute namespace of chunk
    // names so relative requires resolve against the requiring file.
    let project = Project::new("relative-require", &[]);
    std::fs::create_dir_all(project.dir.join("sub")).unwrap();
    std::fs::write(project.dir.join("sub/mod.luau"), "return 42").unwrap();
    std::fs::write(
        project.dir.join("sub/main.luau"),
        "local n = require('./mod')\nassert(n == 42)\n",
    )
    .unwrap();
    let (code, _, stderr) = project.run(&PathBuf::from("sub/main.luau"), &["--quiet"]);
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn attributed_colored_output_for_two_sessions() {
    let project = Project::new("color", &[("MOCK_CHUNKS", "hello world")]);
    let script = project.script(
        r#"
local agent = ptah.agent("mock")
local s1 = agent:session()
local s2 = agent:session()
local r1 = ptah.spawn(function() return s1:prompt("one") end)
local r2 = ptah.spawn(function() return s2:prompt("two") end)
r1:await(); r2:await()
"#,
    );
    let (code, stdout, _) = project.run(&script, &[]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("[mock/s1]"),
        "missing s1 attribution:\n{stdout}"
    );
    assert!(
        stdout.contains("[mock/s2]"),
        "missing s2 attribution:\n{stdout}"
    );
    assert!(
        stdout.contains("\x1b["),
        "expected ANSI color codes:\n{stdout}"
    );
    assert!(stdout.contains("hello world"));
}

#[test]
fn no_color_keeps_prefixes_without_ansi() {
    let project = Project::new("nocolor", &[("MOCK_CHUNKS", "plain text")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--no-color"]);
    let stdout = common::strip_timestamps(&stdout);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("[mock/s1]"), "{stdout}");
    assert!(!stdout.contains('\x1b'), "no ANSI expected:\n{stdout}");
}

#[test]
fn quiet_suppresses_render_but_not_print() {
    let project = Project::new("quiet", &[("MOCK_CHUNKS", "streamed")]);
    let script = project.script(
        r#"
print("script-said-this")
ptah.log("log-line")
local s = ptah.agent("mock"):session()
s:prompt("hi")
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--quiet"]);
    let stdout = common::strip_timestamps(&stdout);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("script-said-this"), "{stdout}");
    assert!(stdout.contains("[ptah] log-line"), "{stdout}");
    assert!(!stdout.contains("[mock/"), "{stdout}");
}

#[test]
fn double_verbose_passes_agent_stderr_through() {
    let project = Project::new("stderr", &[("MOCK_STDERR", "agent-chatter")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
"#,
    );
    let (code, stdout, _) = project.run(&script, &["-vv"]);
    let stdout = common::strip_timestamps(&stdout);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("agent-chatter"),
        "agent stderr not passed through:\n{stdout}"
    );

    // Without -vv the chatter stays hidden.
    let (code, stdout, _) = project.run(&script, &[]);
    assert_eq!(code, 0);
    assert!(!stdout.contains("agent-chatter"), "{stdout}");
}

#[test]
fn usage_update_renders_context_line() {
    // agent-sessions spec: `usage_update` carries context-window used/size
    // and is rendered for display (token counts come from the prompt
    // response and are asserted in the acp tests).
    let project = Project::new("usage", &[("MOCK_USAGE", "5,10,2,3")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
"#,
    );
    let (code, stdout, _) = project.run(&script, &[]);
    let stdout = common::strip_timestamps(&stdout);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("context: 5/10 tokens"),
        "usage_update not rendered:\n{stdout}"
    );
}

#[test]
fn exit_code_propagates() {
    let project = Project::new("exit", &[]);
    let script = project.script("ptah.exit(3)");
    let (code, _, _) = project.run(&script, &[]);
    assert_eq!(code, 3);

    let script = project.script("error('boom', 0)");
    let (code, _, stderr) = project.run(&script, &[]);
    assert_eq!(code, 1);
    assert!(stderr.contains("boom"), "{stderr}");
}

#[test]
fn config_change_lifecycle_lines_render() {
    // session-config-options spec "Config changes are rendered": a
    // successful setConfig and an agent-pushed config_option_update each
    // render one session-attributed lifecycle line naming the changed
    // option id and its new value (--verbose).
    let dir = std::env::temp_dir().join(format!("ptah-cli-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();
    // Env values carry JSON: TOML literal strings keep the quotes intact.
    let config = format!(
        "[agents.mock]\ncommand = \"{}\"\nargs = []\n\n[agents.mock.env]\nMOCK_CONFIG_OPTIONS = '[{}]'\nMOCK_CONFIG_UPDATE = '[{}]'\n",
        mock_bin(),
        r#"{"id":"model","name":"Model","type":"select","currentValue":"opus","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]}"#,
        r#"{"id":"model","name":"Model","type":"select","currentValue":"sonnet","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"},{"value":"sonnet","name":"Sonnet"}]}"#,
    );
    std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();
    let script = dir.join("main.luau");
    std::fs::write(
        &script,
        r#"
local s = ptah.agent("mock"):session()
s:prompt("first")             -- the agent pushes model=sonnet after this turn
s:setConfig("model", "haiku") -- and the set reports model=haiku
s:prompt("second")
s:close()
"#,
    )
    .unwrap();
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .arg("--verbose")
        .current_dir(&dir)
        .output()
        .expect("run ptah");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stripped = common::strip_timestamps(&stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stripped.contains("[ptah] mock/s1: config changed: model=sonnet"),
        "pushed change not rendered:\n{stdout}"
    );
    assert!(
        stripped.contains("[ptah] mock/s1: config changed: model=haiku"),
        "set change not rendered:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Tool-line rendering (agent-sessions spec: the tool-line contract)
// ---------------------------------------------------------------------------

/// Bodies of the rendered `tool: …` lines, timestamps stripped. Runs must
/// use `--no-color` so the `] tool: ` split is exact.
fn tool_bodies(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(common::strip_timestamp)
        .filter_map(|l| l.split_once("] tool: ").map(|(_, body)| body.to_string()))
        .collect()
}

/// The seconds value of a terminal line's `(status, X.Ys)` suffix.
fn terminal_duration(body: &str, status: &str) -> f64 {
    let rest = body
        .rsplit_once(&format!("({status}, "))
        .unwrap_or_else(|| panic!("no ({status}, …) suffix: {body:?}"))
        .1;
    let rest = rest
        .strip_suffix(')')
        .unwrap_or_else(|| panic!("no closing paren: {rest:?}"));
    let secs = rest
        .strip_suffix('s')
        .unwrap_or_else(|| panic!("no s unit: {rest:?}"));
    // `Mm SS.Ss` above a minute: take both parts.
    if let Some((mins, s)) = secs.split_once("m ") {
        mins.trim().parse::<f64>().unwrap() * 60.0 + s.parse::<f64>().unwrap()
    } else {
        secs.parse::<f64>().unwrap()
    }
}

fn run_prompt_script(project: &Project, flags: &[&str]) -> String {
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );
    let (code, stdout, _) = project.run(&script, flags);
    assert_eq!(code, 0, "{stdout}");
    stdout
}

#[test]
fn tool_line_renders_start_and_terminal_only() {
    // agent-sessions spec "Tool call renders start and terminal lines" +
    // "Repeated statuses do not flood the log": the full
    // pending → in_progress → in_progress → completed sequence renders
    // exactly two lines — the bare title at start, title + status +
    // duration at completion. The repeated in_progress renders nothing.
    let project = Project::new(
        "tool-flow",
        &[(
            "MOCK_TOOL_FLOW",
            "pending,in_progress,in_progress,completed",
        )],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    let lines = tool_bodies(&stdout);
    assert_eq!(lines.len(), 2, "exactly start + terminal:\n{stdout}");
    assert_eq!(lines[0], "Search files \"foo\"", "start line:\n{stdout}");
    assert!(
        lines[1].starts_with("Search files \"foo\" (completed, ") && lines[1].ends_with("s)"),
        "terminal line:\n{stdout}"
    );
    // Titles resolve through the id→title map: never the raw call id.
    assert!(
        !stdout.contains("tool-flow-1"),
        "raw call id leaked:\n{stdout}"
    );
}

#[test]
fn tool_line_re_sent_terminal_status_is_silent() {
    // agent-sessions spec "Repeated statuses do not flood the log": a
    // re-sent terminal status renders nothing beyond its first line.
    let project = Project::new(
        "tool-repeat",
        &[("MOCK_TOOL_FLOW", "pending,completed,completed")],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    let lines = tool_bodies(&stdout);
    assert_eq!(lines.len(), 1, "only the first terminal renders:\n{stdout}");
    assert!(
        lines[0].starts_with("Search files \"foo\" (completed, "),
        "terminal line:\n{stdout}"
    );
}

#[test]
fn tool_line_pending_is_silent() {
    // agent-sessions spec "Pending is silent": a pending announcement
    // renders no line at all.
    let project = Project::new("tool-pending", &[("MOCK_TOOL_FLOW", "pending")]);
    let stdout = run_prompt_script(&project, &["--no-color"]);
    assert!(
        tool_bodies(&stdout).is_empty(),
        "pending must not render:\n{stdout}"
    );
}

#[test]
fn tool_line_unannounced_update_falls_back_to_call_id() {
    // agent-sessions spec "Unannounced update falls back to the call id":
    // `!`-prefixed entries arrive without any `tool_call` announcement,
    // so the lines name the raw id.
    let project = Project::new(
        "tool-unannounced",
        &[("MOCK_TOOL_FLOW", "!in_progress,!completed")],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    let lines = tool_bodies(&stdout);
    assert_eq!(lines.len(), 2, "start + terminal still render:\n{stdout}");
    assert_eq!(lines[0], "tool-flow-1", "raw-id start line:\n{stdout}");
    assert!(
        lines[1].starts_with("tool-flow-1 (completed, ") && lines[1].ends_with("s)"),
        "raw-id terminal line:\n{stdout}"
    );
}

#[test]
fn tool_line_direct_completion_measures_from_first_observation() {
    // agent-sessions spec "Direct completion without progress": pending
    // → completed with no in_progress renders only the terminal line,
    // duration measured from first observation (the pending
    // announcement) — MOCK_DELAY_MS=120 guarantees a non-trivial span.
    let project = Project::new(
        "tool-direct",
        &[
            ("MOCK_TOOL_FLOW", "pending,completed"),
            ("MOCK_DELAY_MS", "120"),
        ],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    let lines = tool_bodies(&stdout);
    assert_eq!(lines.len(), 1, "terminal line only:\n{stdout}");
    assert!(
        lines[0].starts_with("Search files \"foo\" (completed, "),
        "terminal line:\n{stdout}"
    );
    assert!(
        terminal_duration(&lines[0], "completed") >= 0.1,
        "duration must span the announcement → completion gap:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Timestamps (render-logging / cli spec: rendered lines carry a full
// date timestamp)
// ---------------------------------------------------------------------------

/// `true` for `yyyy-mm-dd HH:MM:SS` at the start of a (plain) line.
fn starts_with_dated_ts(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() >= 19
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b' '
        && b[11..13].iter().all(u8::is_ascii_digit)
        && b[13] == b':'
        && b[14..16].iter().all(u8::is_ascii_digit)
        && b[16] == b':'
        && b[17..19].iter().all(u8::is_ascii_digit)
}

#[test]
fn rendered_lines_carry_plain_timestamps_under_no_color() {
    // cli spec "Rendered lines are timestamped" + "No-color keeps plain
    // timestamps": every renderer line (prompt, chunk, tool, plan, usage,
    // ptah.log) begins `yyyy-mm-dd HH:MM:SS [` with no ANSI anywhere;
    // script print output is byte-identical.
    let project = Project::new(
        "ts-plain",
        &[
            ("MOCK_CHUNKS", "hello"),
            ("MOCK_TOOL", "1"),
            ("MOCK_PLAN", "1"),
            ("MOCK_USAGE", "5,10,2,3"),
        ],
    );
    let script = project.script(
        r#"
print("print-line")
ptah.log("log-line")
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "{stdout}");
    let mut saw_print = false;
    let mut saw_rendered = false;
    for line in stdout.lines() {
        if line == "print-line" {
            saw_print = true; // "Script print output is untouched"
            continue;
        }
        saw_rendered = true;
        assert!(
            starts_with_dated_ts(line) && line[19..].starts_with(" ["),
            "line misses `yyyy-mm-dd HH:MM:SS [` prefix: {line:?}\n{stdout}"
        );
    }
    assert!(saw_print, "print line missing:\n{stdout}");
    assert!(saw_rendered, "no rendered lines found:\n{stdout}");
    assert!(!stdout.contains('\x1b'), "no ANSI expected:\n{stdout}");
}

#[test]
fn rendered_lines_carry_dimmed_timestamps_under_color() {
    // Same contract with color on: the timestamp is dimmed (`\x1b[2m` …
    // `\x1b[0m`) ahead of the `[label]` prefix.
    let project = Project::new("ts-color", &[("MOCK_CHUNKS", "hello")]);
    let stdout = run_prompt_script(&project, &[]);
    let chunk_line = stdout
        .lines()
        .find(|l| l.ends_with("hello\x1b[0m"))
        .unwrap_or_else(|| panic!("chunk line missing:\n{stdout}"));
    assert!(
        chunk_line.starts_with("\x1b[2m")
            && starts_with_dated_ts(&chunk_line[4..])
            && chunk_line[23..].starts_with("\x1b[0m ["),
        "timestamp not dimmed-leading: {chunk_line:?}\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Prompt lines (render-logging spec: "Prompt turns render a prompt line")
// ---------------------------------------------------------------------------

/// Bodies of the rendered `prompt: …` lines, timestamps stripped.
fn prompt_bodies(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(common::strip_timestamp)
        .filter_map(|l| l.split_once("] prompt: ").map(|(_, body)| body.to_string()))
        .collect()
}

#[test]
fn prompt_line_renders_once_per_turn_with_attribution() {
    // One `prompt:` line per turn, at send time (before the turn's other
    // output), attributed to the sending session, whitespace collapsed.
    let project = Project::new("prompt-line", &[("MOCK_CHUNKS", "answer")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt("review\n  the   auth module\nfor drift")
s:prompt("second turn")
s:close()
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        prompt_bodies(&stdout),
        vec![
            "review the auth module for drift".to_string(),
            "second turn".to_string(),
        ],
        "exactly one prompt line per turn, collapsed:\n{stdout}"
    );
    // The first prompt line carries session attribution and precedes the
    // turn's streamed output.
    let stripped: Vec<&str> = stdout.lines().map(common::strip_timestamp).collect();
    let prompt_at = stripped
        .iter()
        .position(|l| *l == "[mock/s1] prompt: review the auth module for drift")
        .unwrap_or_else(|| panic!("attributed prompt line missing:\n{stdout}"));
    let chunk_at = stripped
        .iter()
        .position(|l| *l == "[mock/s1] answer")
        .unwrap_or_else(|| panic!("chunk line missing:\n{stdout}"));
    assert!(
        prompt_at < chunk_at,
        "prompt line must precede output:\n{stdout}"
    );
}

#[test]
fn prompt_line_truncates_long_prompts() {
    // "Long prompt truncated": the first budget's worth of collapsed
    // text followed by `…` and no more.
    let project = Project::new("prompt-trunc", &[("MOCK_CHUNKS", "ok")]);
    let script = project.script(
        r#"
local s = ptah.agent("mock"):session()
s:prompt(string.rep("a", 130) .. " tail")
s:close()
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        prompt_bodies(&stdout),
        vec![format!("{}…", "a".repeat(120))],
        "budget + marker, nothing more:\n{stdout}"
    );
}

#[test]
fn quiet_suppresses_the_prompt_line() {
    let project = Project::new("prompt-quiet", &[("MOCK_CHUNKS", "streamed")]);
    let script = project.script(
        r#"
print("script-said-this")
local s = ptah.agent("mock"):session()
s:prompt("hi")
s:close()
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--quiet"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        !stdout.contains("prompt:"),
        "quiet must suppress:\n{stdout}"
    );
    assert!(
        stdout.contains("script-said-this"),
        "print survives:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Tool input peeks (render-logging spec: "Tool lines carry an input peek"
// and "Peek paths render session-relative")
// ---------------------------------------------------------------------------

#[test]
fn execute_peek_appends_the_command() {
    // "Execute kind shows the command": `tool: bash git status` and the
    // terminal line carrying the same peek.
    let project = Project::new(
        "peek-exec",
        &[
            ("MOCK_TOOL_FLOW", "pending,in_progress,completed"),
            ("MOCK_TOOL_TITLE", "bash"),
            ("MOCK_TOOL_KIND", "execute"),
            ("MOCK_TOOL_RAW_INPUT", "{\"command\": \"git status\"}"),
        ],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    let lines = tool_bodies(&stdout);
    assert_eq!(lines.len(), 2, "start + terminal:\n{stdout}");
    assert_eq!(lines[0], "bash git status", "start line:\n{stdout}");
    assert!(
        lines[1].starts_with("bash git status (completed, ") && lines[1].ends_with("s)"),
        "terminal line:\n{stdout}"
    );
}

#[test]
fn read_peek_shortens_path_under_session_cwd() {
    // "Read kind shows the location path" + "Path under session cwd":
    // the default session cwd is the invocation dir (= the project dir).
    let project = Project::new("peek-read", &[]);
    project.set_env(&[
        ("MOCK_TOOL_FLOW", "pending,in_progress"),
        ("MOCK_TOOL_TITLE", "read"),
        ("MOCK_TOOL_KIND", "read"),
        (
            "MOCK_TOOL_LOCATIONS",
            &format!("{}/src/render/mod.rs:118", project.dir.display()),
        ),
    ]);
    let stdout = run_prompt_script(&project, &["--no-color"]);
    assert_eq!(
        tool_bodies(&stdout),
        vec!["read src/render/mod.rs:118".to_string()],
        "cwd-relative path with :line:\n{stdout}"
    );
}

#[test]
fn peek_path_outside_cwd_collapses_under_home() {
    // "Path outside session cwd but under home" → `~/notes/todo.md`,
    // while a path outside home entirely stays as received.
    let under_home = format!("{}/notes/todo.md", home_dir_string());
    let project = Project::new(
        "peek-home",
        &[
            ("MOCK_TOOL_FLOW", "pending,in_progress"),
            ("MOCK_TOOL_TITLE", "edit"),
            ("MOCK_TOOL_KIND", "edit"),
            ("MOCK_TOOL_LOCATIONS", &under_home),
        ],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    assert_eq!(
        tool_bodies(&stdout),
        vec!["edit ~/notes/todo.md".to_string()],
        "~-collapsed path:\n{stdout}"
    );

    let project = Project::new(
        "peek-abs",
        &[
            ("MOCK_TOOL_FLOW", "pending,in_progress"),
            ("MOCK_TOOL_TITLE", "read"),
            ("MOCK_TOOL_KIND", "read"),
            ("MOCK_TOOL_LOCATIONS", "/tmp/build.log"),
        ],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    assert_eq!(
        tool_bodies(&stdout),
        vec!["read /tmp/build.log".to_string()],
        "outside home stays as received:\n{stdout}"
    );
}

#[test]
fn unknown_kind_falls_back_to_compact_raw_input_json() {
    // "Unknown tool falls back to compact raw input".
    let project = Project::new(
        "peek-json",
        &[
            ("MOCK_TOOL_FLOW", "pending,in_progress"),
            ("MOCK_TOOL_TITLE", "grep"),
            ("MOCK_TOOL_RAW_INPUT", "{\"pattern\": \"foo\"}"),
        ],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    assert_eq!(
        tool_bodies(&stdout),
        vec!["grep {\"pattern\":\"foo\"}".to_string()],
        "compact JSON peek:\n{stdout}"
    );
}

#[test]
fn peek_contained_in_the_title_is_not_duplicated() {
    // "Title already contains the peek": pi-acp-style bash titles are the
    // command itself, so the peek must not append a duplicate.
    let project = Project::new(
        "peek-dedup",
        &[
            ("MOCK_TOOL_FLOW", "pending,in_progress,completed"),
            ("MOCK_TOOL_TITLE", "git status"),
            ("MOCK_TOOL_KIND", "execute"),
            ("MOCK_TOOL_RAW_INPUT", "{\"command\": \"git status\"}"),
        ],
    );
    let stdout = run_prompt_script(&project, &["--no-color"]);
    let lines = tool_bodies(&stdout);
    assert_eq!(lines.len(), 2, "start + terminal:\n{stdout}");
    assert_eq!(lines[0], "git status", "no duplication on start:\n{stdout}");
    assert!(
        lines[1].starts_with("git status (completed, "),
        "no duplication on terminal:\n{stdout}"
    );
}

#[test]
fn readme_output_example_lines_match_e2e_output() {
    // The README "Output format" example, reproduced line-for-line
    // (timestamps stripped; durations are live so only prefixes are
    // exact there). Two mock agents stand in for one claude session's
    // bash + read activity: knob env is per agent process.
    let dir = std::env::temp_dir().join(format!("ptah-cli-doc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();
    let config = format!(
        "[agents.bash]\ncommand = \"{mock}\"\nargs = []\n\
         \n[agents.bash.env]\nMOCK_TOOL_FLOW = 'pending,in_progress,completed'\n\
         MOCK_TOOL_TITLE = 'bash'\nMOCK_TOOL_KIND = 'execute'\n\
         MOCK_TOOL_RAW_INPUT = '{{\"command\": \"git status\"}}'\n\
         \n[agents.read]\ncommand = \"{mock}\"\nargs = []\n\
         \n[agents.read.env]\nMOCK_TOOL_FLOW = 'pending,in_progress'\n\
         MOCK_TOOL_TITLE = 'read'\nMOCK_TOOL_KIND = 'read'\n\
         MOCK_CHUNKS = 'Looks fine — two nits below.'\n\
         MOCK_TOOL_LOCATIONS = '{dir}/src/render/mod.rs:118'\n",
        mock = mock_bin(),
        dir = dir.display(),
    );
    std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();
    let script = dir.join("main.luau");
    std::fs::write(
        &script,
        r#"
local bash = ptah.agent("bash"):session()
local read = ptah.agent("read"):session()
local rb = ptah.spawn(function() return bash:prompt("review the auth module for drift against the spec") end)
local rr = ptah.spawn(function() return read:prompt("review the auth module for drift against the spec") end)
rb:await(); rr:await()
ptah.log("log line from ptah.log")
bash:close(); read:close()
"#,
    )
    .unwrap();
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .arg("--no-color")
        .current_dir(&dir)
        .output()
        .expect("run ptah");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{stdout}");
    let bodies: Vec<String> = stdout
        .lines()
        .map(common::strip_timestamp)
        .map(|l| {
            l.split_once("] ")
                .map_or_else(|| l.to_string(), |(_, b)| b.to_string())
        })
        .collect();
    // Each example line appears verbatim (durations are live, so the
    // terminal line matches by prefix). The two fixture sessions
    // interleave freely — ordering within one call is pinned by the
    // dedicated tool-line tests above.
    let expected = [
        "prompt: review the auth module for drift against the spec",
        "tool: bash git status",
        "tool: bash git status (completed, ",
        "tool: read src/render/mod.rs:118",
        "Looks fine — two nits below.",
        "log line from ptah.log",
    ];
    for want in expected {
        assert!(
            bodies.iter().any(|b| b.starts_with(want)),
            "doc example line missing: {want:?}\n{stdout}"
        );
    }
}

/// `$HOME` as a display string (peek path fixtures).
fn home_dir_string() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/home".to_string())
}

// ---------------------------------------------------------------------------
// ptah.exec rendering + environment (shell-exec capability)
// ---------------------------------------------------------------------------

#[test]
fn exec_lifecycle_lines_render_in_order_around_a_slow_command() {
    // render-logging "Exec lines render command and outcome" + "Output
    // is not echoed": a start line when the command launches, an end
    // line with exit code and duration when it settles — timestamped
    // like every other line and attributed as script activity
    // (`[ptah]`), in order around the call. The payload is built by
    // adjacent-string concatenation (`'ca''ptured-payload'`) so its
    // text never appears in the command string itself — proving the
    // terminal shows only lifecycle lines, never captured output.
    let project = Project::new("exec-lines", &[]);
    let script = project.script(
        r#"
ptah.log("before")
local r = ptah.exec("sleep 0.3; printf 'ca''ptured-payload'")
assert(r.exitCode == 0 and r.stdout == "captured-payload", "stdout: " .. r.stdout)
ptah.log("after")
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--no-color"]);
    assert_eq!(code, 0, "{stdout}");
    let stripped = common::strip_timestamps(&stdout);
    // Captured output is the script's, not the terminal's: the payload
    // (distinct from the command text the lifecycle lines echo) must
    // not appear anywhere in the rendered output.
    assert!(
        !stripped.contains("captured-payload"),
        "captured child output must not be echoed:\n{stripped}"
    );
    let lines: Vec<&str> = stripped.lines().collect();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("line with {needle:?} missing:\n{stripped}"))
    };
    assert_eq!(
        lines[at("exec: sleep 0.3")],
        "[ptah] exec: sleep 0.3; printf 'ca''ptured-payload'",
        "{stripped}"
    );
    let end = at("(exit 0,");
    assert!(
        lines[end].starts_with("[ptah] exec: sleep 0.3; printf 'ca''ptured-payload' (exit 0, "),
        "end line: {}",
        lines[end]
    );
    assert!(at("before") < at("exec: sleep 0.3"), "order:\n{stripped}");
    assert!(at("exec: sleep 0.3") < end, "order:\n{stripped}");
    assert!(end < at("after"), "order:\n{stripped}");
}

#[test]
fn quiet_suppresses_exec_lines() {
    // render-logging "Quiet suppresses exec lines": exec lines are
    // session-event-like, not script logs — `--quiet` drops them, while
    // `ptah.log` output survives.
    let project = Project::new("exec-quiet", &[]);
    let script = project.script(
        r#"
ptah.log("log-line")
local r = ptah.exec("printf hi")
assert(r.exitCode == 0)
"#,
    );
    let (code, stdout, _) = project.run(&script, &["--quiet"]);
    let stdout = common::strip_timestamps(&stdout);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("[ptah] log-line"), "{stdout}");
    assert!(
        !stdout.contains("exec:"),
        "quiet must drop exec lines:\n{stdout}"
    );
    assert!(!stdout.contains("printf hi"), "{stdout}");
}

#[test]
fn exec_inherits_env_and_cwd() {
    // shell-exec "Environment inheritance" + "Working directory
    // inheritance": the child sees ptah's env and runs in the
    // invocation directory (marker.txt lives in the project dir).
    let project = Project::new("exec-env", &[]);
    std::fs::write(project.dir.join("marker.txt"), "from-cwd\n").unwrap();
    let script = project.script(
        r#"
local r = ptah.exec("printf %s $PTAH_EXEC_TOKEN; cat marker.txt")
assert(r.exitCode == 0, "exit: " .. tostring(r.exitCode) .. " " .. r.stderr)
assert(r.stdout == "tok-7from-cwd\n", "stdout: " .. r.stdout)
"#,
    );
    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script)
        .current_dir(&project.dir)
        .env("PTAH_EXEC_TOKEN", "tok-7")
        .output()
        .expect("run ptah");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Drive one `ptah run` of the park script, deliver `sig` to the
/// process once it reports parked (an exec is in flight), and check the
/// cancelled run: exit code, no orphaned exec child, and (for the
/// color check) the rendered lines.
#[cfg(unix)]
fn signal_cancels_the_run(sig: i32, expected_code: i32, tag: &str, extra_flags: &[&str]) -> String {
    use std::io::BufRead;
    use std::process::Stdio;

    // A previously failed run may have leaked this tag's sleep; sweep
    // it first so the final no-orphan assertion is about *this* run.
    common::kill_processes(tag);
    common::wait_for_processes(tag, 0, "stale tag cleared before the run");

    let project = Project::new(&format!("signal-{tag}"), &[]);
    let script = project.script(&format!(
        r#"
ptah.spawn(function() return ptah.exec("sleep {tag}") end)
ptah.sleep(200)
ptah.log("parked")
ptah.sleep(60000)
"#
    ));
    let mut child = std::process::Command::new(ptah_bin())
        .arg("run")
        .args(extra_flags)
        .arg(&script)
        .current_dir(&project.dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ptah");

    // Read stdout on a thread so the signal can fire exactly when the
    // script is parked (a blocking read here could never time out).
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("piped stdout");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if tx.send(line.expect("read stdout")).is_err() {
                break;
            }
        }
    });
    let mut seen = String::new();
    loop {
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("script never parked");
        seen.push_str(&line);
        seen.push('\n');
        if line.contains("parked") {
            break;
        }
    }
    assert!(
        seen.contains(&format!("exec: sleep {tag}")),
        "exec must be in flight before the signal:\n{seen}"
    );

    // The signal reaches ptah only — the exec child runs in its own
    // process group, out of a terminal signal's reach — so a surviving
    // child after this would be ptah's teardown failing, not the OS.
    unsafe { libc::kill(child.id() as i32, sig) };

    // Drain remaining lines (pipe closes at exit) and reap the status.
    while let Ok(line) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
        seen.push_str(&line);
        seen.push('\n');
    }
    let status = child.wait().expect("wait ptah");
    assert_eq!(
        status.code(),
        Some(expected_code),
        "cancelled run must exit {expected_code}; output:\n{seen}"
    );
    common::wait_for_processes(tag, 0, "signal teardown killed the in-flight exec");
    seen
}

#[cfg(unix)]
#[test]
fn sigint_cancels_the_run_kills_in_flight_exec_and_exits_130() {
    // shell-exec "In-flight execs are killed at teardown", the outer
    // cancel leg: SIGINT must ride the run's teardown (kill the exec
    // group, close sessions) and exit 130 — previously the process died
    // on the signal with no teardown and the exec child was orphaned.
    signal_cancels_the_run(libc::SIGINT, 130, "8841", &["--no-color"]);
}

#[cfg(unix)]
#[test]
fn sigterm_cancels_the_run_and_exits_143() {
    signal_cancels_the_run(libc::SIGTERM, 143, "8842", &["--no-color"]);
}

#[cfg(unix)]
#[test]
fn sigint_exec_lines_render_in_color_mode() {
    // render-logging "Color mode shows both lines": default (color)
    // output still carries the exec lines — timestamped and ANSI-dimmed
    // like every other rendered line, never dropped for color reasons.
    let seen = signal_cancels_the_run(libc::SIGINT, 130, "8843", &[]);
    let line = seen
        .lines()
        .find(|l| l.contains("exec: sleep 8843") && !l.contains("("))
        .expect("start line present in color mode");
    assert!(
        line.contains("\u{1b}["),
        "color mode must ANSI-style the exec line: {line:?}"
    );
}

#[cfg(unix)]
#[test]
fn sigint_during_task_drain_cancels_run_and_kills_in_flight_exec() {
    // Regression: the shutdown watch was once raced only against the
    // script body, so a SIGINT arriving while spawned tasks drain (the
    // window after the body ends — here a task parked in a no-budget
    // exec, which keeps the window open indefinitely) was swallowed,
    // and only the second signal's hard exit could end the run,
    // skipping teardown and orphaning the exec child. The first signal
    // must ride the same teardown path as during the body: exit 130,
    // child dead, no orphan.
    use std::io::BufRead;
    use std::process::Stdio;

    let tag = "8845";
    common::kill_processes(tag);
    common::wait_for_processes(tag, 0, "stale tag cleared before the run");

    // Unlike signal_cancels_the_run's script, the main body ENDS right
    // after reporting parked — the signal lands in the task drain.
    let project = Project::new("signal-drain", &[]);
    let script = project.script(&format!(
        r#"
ptah.spawn(function() return ptah.exec("sleep {tag}") end)
ptah.sleep(200)
ptah.log("parked")
"#
    ));
    let mut child = std::process::Command::new(ptah_bin())
        .arg("run")
        .arg("--no-color")
        .arg(&script)
        .current_dir(&project.dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ptah");

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("piped stdout");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if tx.send(line.expect("read stdout")).is_err() {
                break;
            }
        }
    });
    let mut seen = String::new();
    loop {
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("script never parked");
        seen.push_str(&line);
        seen.push('\n');
        if line.contains("parked") {
            break;
        }
    }
    assert!(
        seen.contains(&format!("exec: sleep {tag}")),
        "exec must be in flight before the signal:\n{seen}"
    );
    // Let the body finish and the run settle into the drain.
    std::thread::sleep(std::time::Duration::from_millis(250));

    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };

    // Bounded reap: under the bug the run never exits on one signal
    // (the parked exec has no budget), so the regression must fail
    // here rather than hang the suite.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait ptah") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "run ignored the first SIGINT during the task drain (output so far):\n{seen}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    while let Ok(line) = rx.recv_timeout(std::time::Duration::from_secs(1)) {
        seen.push_str(&line);
        seen.push('\n');
    }
    assert_eq!(
        status.code(),
        Some(130),
        "drain-cancelled run must exit 130; output:\n{seen}"
    );
    common::wait_for_processes(tag, 0, "drain teardown killed the in-flight exec");
}

/// Drive one `ptah run` with a hung agent turn (MOCK_HANG: the agent
/// never finishes the prompt on its own), deliver `sig` once the turn
/// is in flight, and check the cancelled run: exit code and — the leg
/// today's signal tests never covered — no surviving agent process.
/// The agent's argv carries `tag` so /proc can find exactly this run's
/// agent (the mock ignores unknown args).
#[cfg(unix)]
fn signal_cancels_the_run_killing_the_agent(sig: i32, expected_code: i32, name: &str, tag: &str) {
    use std::io::BufRead;
    use std::process::Stdio;

    // A previously failed run may have leaked this tag's agent; sweep
    // it first so the final no-survivor assertion is about *this* run.
    common::kill_processes(tag);
    common::wait_for_processes(tag, 0, "stale tag cleared before the run");

    // `name` (not the tag) keys the dir: ptah's own cmdline must stay
    // tag-free so /proc counts exactly the agent.
    let dir = std::env::temp_dir().join(format!(
        "ptah-cli-signal-agent-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();
    std::fs::write(
        dir.join(".ptah").join("config.toml"),
        format!(
            "[agents.mock]\ncommand = \"{}\"\nargs = [\"{tag}\"]\n\n[agents.mock.env]\nMOCK_HANG = '1'\n",
            mock_bin()
        ),
    )
    .unwrap();
    let script = dir.join("main.luau");
    std::fs::write(
        &script,
        "local s = ptah.agent(\"mock\"):session()\ns:prompt(\"hang\")\n",
    )
    .unwrap();

    let mut child = std::process::Command::new(ptah_bin())
        .arg("run")
        .arg("--no-color")
        .arg(&script)
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ptah");

    // The prompt line renders at send time, so it proves the turn (and
    // the agent behind it) is in flight before the signal lands.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("piped stdout");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if tx.send(line.expect("read stdout")).is_err() {
                break;
            }
        }
    });
    let mut seen = String::new();
    loop {
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("agent turn never started");
        seen.push_str(&line);
        seen.push('\n');
        if line.contains("prompt: hang") {
            break;
        }
    }
    assert_eq!(
        common::count_processes(tag),
        1,
        "agent must be alive and tagged before the signal"
    );

    // The signal reaches ptah only — the agent runs in its own process
    // group, out of a terminal signal's reach — so a surviving agent
    // after this would be ptah's teardown failing, not the OS.
    unsafe { libc::kill(child.id() as i32, sig) };

    while let Ok(line) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
        seen.push_str(&line);
        seen.push('\n');
    }
    let status = child.wait().expect("wait ptah");
    assert_eq!(
        status.code(),
        Some(expected_code),
        "cancelled run must exit {expected_code}; output:\n{seen}"
    );
    common::wait_for_processes(tag, 0, "signal teardown killed the agent");
}

#[cfg(unix)]
#[test]
fn sigint_cancels_the_run_kills_the_agent_and_exits_130() {
    // agent-sessions "Processes are torn down at run end", the Ctrl-C
    // leg: the agent sits in its own process group where the terminal
    // signal cannot reach it; teardown must kill and reap it before
    // the 130 exit (SIGTERM likewise → 143).
    signal_cancels_the_run_killing_the_agent(libc::SIGINT, 130, "int", "8846");
}

#[cfg(unix)]
#[test]
fn sigterm_cancels_the_run_killing_the_agent_and_exits_143() {
    signal_cancels_the_run_killing_the_agent(libc::SIGTERM, 143, "term", "8847");
}

#[cfg(unix)]
#[test]
fn second_signal_sweeps_agents_and_execs_then_exits_with_its_code() {
    // The force escape: a first signal starts teardown, a second one
    // lands while teardown is still draining (the exec drain and the
    // session joins take ~7-11ms; the monitor re-arms within
    // microseconds of the first signal, so a short gap keeps the
    // second inside the window). No destructor runs on the hard exit —
    // the sweep must kill the hung agent's group and the tagged exec
    // group itself, and the exit code must match the *second* signal
    // (SIGTERM → 143, previously hardcoded 130). If teardown happens
    // to win the race the assertions still hold: the children are
    // just as dead, killed by teardown instead.
    use std::io::BufRead;
    use std::process::Stdio;

    let agent_tag = "8848";
    let exec_tag = "8849";
    common::kill_processes(agent_tag);
    common::kill_processes(exec_tag);
    common::wait_for_processes(agent_tag, 0, "stale agent tag cleared");
    common::wait_for_processes(exec_tag, 0, "stale exec tag cleared");

    let dir = std::env::temp_dir().join(format!("ptah-cli-signal-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();
    std::fs::write(
        dir.join(".ptah").join("config.toml"),
        format!(
            "[agents.mock]\ncommand = \"{}\"\nargs = [\"{agent_tag}\"]\n\n[agents.mock.env]\nMOCK_HANG = '1'\n",
            mock_bin()
        ),
    )
    .unwrap();
    let script = dir.join("main.luau");
    std::fs::write(
        &script,
        format!(
            "local s = ptah.agent(\"mock\"):session()\n\
             ptah.spawn(function() return ptah.exec(\"sleep {exec_tag}\") end)\n\
             s:prompt(\"hang\")\n"
        ),
    )
    .unwrap();

    let mut child = std::process::Command::new(ptah_bin())
        .arg("run")
        .arg("--no-color")
        .arg(&script)
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ptah");

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("piped stdout");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if tx.send(line.expect("read stdout")).is_err() {
                break;
            }
        }
    });
    let mut seen = String::new();
    loop {
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("run never parked");
        seen.push_str(&line);
        seen.push('\n');
        // The spawned exec and the main body's prompt race each other
        // onto the screen (either order); both must be in flight before
        // signaling.
        if seen.contains("prompt: hang") && seen.contains(&format!("exec: sleep {exec_tag}")) {
            break;
        }
    }
    assert_eq!(common::count_processes(agent_tag), 1, "agent alive");
    // The exec child may be mid-`execve` (sh exec'ing into sleep),
    // when /proc cmdline reads can come back empty — retry briefly
    // before declaring it alive.
    let mut exec_alive = false;
    for _ in 0..100 {
        if common::count_processes(exec_tag) >= 1 {
            exec_alive = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        exec_alive,
        "exec child alive (sh may have exec'd into sleep)"
    );

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
        // Short gap, sized to the measured teardown window (~7-11ms,
        // floored at 5ms by the exec drain's poll step). Both signals
        // are SIGTERM on purpose: whichever way the race resolves the
        // observable outcome is 143 — the second signal delivered
        // separately fires the sweep and exit(143); the second
        // kernel-coalesced into the first rides normal teardown's
        // 143 — while the old hardcoded-130 hard exit fails the test
        // whenever the second signal lands (the common case at this
        // gap).
        std::thread::sleep(std::time::Duration::from_millis(2));
        libc::kill(child.id() as i32, libc::SIGTERM);
    }

    while let Ok(line) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
        seen.push_str(&line);
        seen.push('\n');
    }
    let status = child.wait().expect("wait ptah");
    assert_eq!(
        status.code(),
        Some(143),
        "the hard exit must carry the second signal's code (SIGTERM → 143); \
         output:\n{seen}"
    );
    common::wait_for_processes(agent_tag, 0, "second-signal sweep killed the agent");
    common::wait_for_processes(exec_tag, 0, "second-signal sweep killed the exec");
}

#[test]
fn teardown_cancelled_exec_renders_start_but_no_end_line() {
    // shell-exec "Exec lifecycle is observable": an exec cancelled by
    // teardown emits no end event — the run's shutdown ended it, not
    // the command's outcome. The start line renders; an end line must
    // not (and the run's own error still reports as itself).
    common::kill_processes("8844");
    common::wait_for_processes("8844", 0, "stale tag cleared before the run");
    let project = Project::new("exec-no-end", &[]);
    let script = project.script(
        r#"
ptah.spawn(function() return ptah.exec("sleep 8844") end)
ptah.sleep(150)
error("script exploded", 0)
"#,
    );
    let (code, stdout, stderr) = project.run(&script, &["--no-color"]);
    let stripped = common::strip_timestamps(&stdout);
    assert_eq!(code, 1, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stripped.contains("[ptah] exec: sleep 8844"),
        "start line must render:\n{stripped}"
    );
    assert!(
        !stripped.contains("sleep 8844 ("),
        "a teardown-cancelled exec must not render an end line:\n{stripped}"
    );
    common::wait_for_processes("8844", 0, "teardown killed the exec");
}
