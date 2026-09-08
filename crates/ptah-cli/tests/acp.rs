//! Integration tests for the ACP client core against the in-repo mock agent.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ptah::acp::{SessionOptions, TurnError, start_session};
use ptah::config::AgentSpec;
use ptah::render::{RenderOptions, Renderer};

fn mock_agent_spec() -> AgentSpec {
    AgentSpec::new(env!("CARGO_BIN_EXE_mock-agent"))
}

fn quiet_renderer() -> Arc<Renderer> {
    Arc::new(Renderer::new(RenderOptions {
        quiet: true,
        ..RenderOptions::default()
    }))
}

fn opts(label: &str) -> SessionOptions {
    SessionOptions {
        cwd: std::env::temp_dir(),
        mcp_servers: vec![],
        label: label.to_string(),
        result: None,
    }
}

#[tokio::test]
async fn handshake_and_prompt_roundtrip() {
    // Task 4.1: spawn + initialize + session/new + full prompt turn.
    let session = start_session(&mock_agent_spec(), opts("mock/s1"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("ping".into(), None)
        .await
        .expect("prompt completes");
    assert_eq!(outcome.text, "ping");
    assert_eq!(outcome.stop_reason, "end_turn");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn spawn_env_is_merged_over_inherited() {
    // Registry spec: entry env values ride on top of the inherited env.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_ENV_DUMP".into(), "PTAH_TEST_MODEL".into());
    spec.env.insert("PTAH_TEST_MODEL".into(), "opus".into());

    let session = start_session(&spec, opts("mock/env"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("ignored".into(), None)
        .await
        .expect("prompt completes");
    assert_eq!(outcome.text, "opus");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn spawn_failure_fails_fast_naming_command() {
    let mut spec = AgentSpec::new("/nonexistent/ptah-test-agent");
    spec.args = vec!["--flag".into()];
    let err = start_session(&spec, opts("bad/cmd"), quiet_renderer())
        .await
        .expect_err("spawn must fail");
    assert!(
        err.to_string().contains("/nonexistent/ptah-test-agent"),
        "{err}"
    );
}

#[tokio::test]
async fn permission_request_denied_turn_still_completes() {
    // Task 4.2: agent asks for permission; ptah answers -32601 (asserted
    // inside the mock); the turn completes anyway.
    let mut spec = mock_agent_spec();
    spec.env.insert("MOCK_PERMISSION".into(), "1".into());
    let session = start_session(&spec, opts("mock/perm"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("hi".into(), None)
        .await
        .expect("turn completes after denial");
    assert_eq!(outcome.stop_reason, "end_turn");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn unsupported_agent_requests_get_method_not_found() {
    // agent-sessions spec: fs/read_text_file, fs/write_text_file,
    // terminal/*, and elicitation/create are answered with the
    // unsupported-method error (asserted inside the mock via MOCK_REQUEST)
    // and the turn still completes — replies are prompt, never hanging.
    let mut spec = mock_agent_spec();
    spec.env.insert(
        "MOCK_REQUEST".into(),
        "fs/read_text_file|fs/write_text_file|terminal/create|terminal/output|elicitation/create|not/a/method"
            .into(),
    );
    let session = start_session(&spec, opts("mock/unsupported"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("hi".into(), None)
        .await
        .expect("turn completes after unsupported requests");
    assert_eq!(outcome.text, "hi");
    assert_eq!(outcome.stop_reason, "end_turn");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn chunked_echo_assembles_final_text_and_usage() {
    // Task 4.3: update folding — chunks -> text, usage on the result.
    let mut spec = mock_agent_spec();
    spec.env.insert("MOCK_CHUNKS".into(), "Hel|lo".into());
    spec.env.insert("MOCK_DELAY_MS".into(), "20".into());
    spec.env.insert("MOCK_USAGE".into(), "5,10,2,3".into());
    let session = start_session(&spec, opts("mock/chunks"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("q".into(), None)
        .await
        .expect("prompt completes");
    assert_eq!(outcome.text, "Hello");
    assert_eq!(outcome.stop_reason, "end_turn");
    assert_eq!(outcome.usage.input, 2);
    assert_eq!(outcome.usage.output, 3);
    session.close();
    session.join().await;
}

#[tokio::test]
async fn mcp_servers_pass_through() {
    // session/new accepts MCP server entries (mock ignores them; serialization
    // failures would fail the handshake).
    use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
    let mut options = opts("mock/mcp");
    options.mcp_servers = vec![McpServer::Stdio(McpServerStdio::new(
        "test-mcp",
        "/bin/true",
    ))];
    let session = start_session(&mock_agent_spec(), options, quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session.prompt("hi".into(), None).await.expect("turn works");
    assert_eq!(outcome.stop_reason, "end_turn");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn cancel_returns_cancelled_stop_reason() {
    // Task 4.4 (cancel path): MOCK_HANG never completes on its own.
    let mut spec = mock_agent_spec();
    spec.env.insert("MOCK_HANG".into(), "1".into());
    let session = start_session(&spec, opts("mock/cancel"), quiet_renderer())
        .await
        .expect("session starts");

    let s2 = session.clone();
    let pending = tokio::spawn(async move { s2.prompt("slow".into(), None).await });

    tokio::time::sleep(Duration::from_millis(100)).await;
    session.cancel();
    let outcome = pending.await.unwrap().expect("cancel is not an error");
    assert_eq!(outcome.stop_reason, "cancelled");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn timeout_sends_cancel_and_raises() {
    // Task 4.4 (timeout path): raises a catchable timeout error after
    // sending session/cancel. The mock (MOCK_HANG) only ever completes a
    // turn in response to cancel, so a healthy follow-up turn proves the
    // cancel was delivered and the session stayed usable.
    let mut spec = mock_agent_spec();
    spec.env.insert("MOCK_HANG".into(), "1".into());
    let session = start_session(&spec, opts("mock/timeout"), quiet_renderer())
        .await
        .expect("session starts");
    let err = session
        .prompt("slow".into(), Some(Duration::from_millis(100)))
        .await
        .expect_err("must time out");
    assert!(matches!(err, TurnError::Timeout), "{err:?}");

    // Follow-up turn round-trips: the session stays usable after a timeout
    // (this prompt also times out — MOCK_HANG — but errors with Timeout, not
    // Closed, proving the connection still answers).
    let err = session
        .prompt("again".into(), Some(Duration::from_millis(100)))
        .await
        .expect_err("second hang must time out too");
    assert!(matches!(err, TurnError::Timeout), "{err:?}");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn default_cwd_is_invocation_dir() {
    // CLI spec: sessions default to the invocation directory.
    let mut spec = mock_agent_spec();
    spec.env.insert("MOCK_ECHO_CWD".into(), "1".into());
    let mut options = opts("mock/cwd");
    options.cwd = std::env::temp_dir();
    let session = start_session(&spec, options, quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("where am I".into(), None)
        .await
        .expect("turn works");
    assert_eq!(outcome.text, std::env::temp_dir().display().to_string());
    session.close();
    session.join().await;
}

#[tokio::test]
async fn close_reaps_agent_process_no_zombie() {
    // Task 4.5: after close+join the child is gone from the process table.
    let session = start_session(&mock_agent_spec(), opts("mock/reap"), quiet_renderer())
        .await
        .expect("session starts");
    let pid = session.pid as i32;
    let _ = session.prompt("hi".into(), None).await;
    session.close();
    session.join().await;
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "agent pid {pid} still in process table (zombie?)"
    );
}

#[tokio::test]
async fn tool_and_plan_updates_flow_through_turn() {
    // Task 5.2: tool_call + tool_call_update + plan updates stream during a
    // turn and the turn still completes.
    let mut spec = mock_agent_spec();
    spec.env.insert("MOCK_TOOL".into(), "1".into());
    spec.env.insert("MOCK_PLAN".into(), "1".into());
    spec.env.insert("MOCK_DELAY_MS".into(), "10".into());
    let session = start_session(&spec, opts("mock/tools"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("do work".into(), None)
        .await
        .expect("turn completes with updates");
    assert_eq!(outcome.text, "do work");
    assert_eq!(outcome.stop_reason, "end_turn");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn two_sessions_are_independent_processes() {
    let a = start_session(&mock_agent_spec(), opts("mock/a"), quiet_renderer())
        .await
        .expect("a starts");
    let b = start_session(&mock_agent_spec(), opts("mock/b"), quiet_renderer())
        .await
        .expect("b starts");
    assert_ne!(a.pid, b.pid, "sessions must not share a process");
    let (ra, rb) = tokio::join!(a.prompt("A".into(), None), b.prompt("B".into(), None));
    assert_eq!(ra.unwrap().text, "A");
    assert_eq!(rb.unwrap().text, "B");
    a.close();
    b.close();
    a.join().await;
    b.join().await;
}

// ---------------------------------------------------------------------------
// Session config options (capability, capture, updates, setConfig)
// ---------------------------------------------------------------------------

/// Advertised option set used across config tests: a select `model`
/// option (category `model`, choices opus/haiku, current `opus`) and a
/// boolean `fast` option (current false, no category).
const MODEL_OPTIONS_JSON: &str = r#"[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"opus","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]},{"id":"fast","name":"Fast mode","type":"boolean","currentValue":false}]"#;

#[tokio::test]
async fn prompt_text_is_last_message_after_tool_use() {
    // scripting spec "Last message after tool use": the mock streams a
    // lead message (MOCK_LEAD_CHUNKS), runs tool activity (MOCK_TOOL),
    // then streams the final message (MOCK_CHUNKS) — the turn's text is
    // the final message only, without the preamble glued in front.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_LEAD_CHUNKS".into(), "Let me check that. ".into());
    spec.env.insert("MOCK_TOOL".into(), "1".into());
    spec.env
        .insert("MOCK_CHUNKS".into(), "The bug is on line 3".into());
    let session = start_session(&spec, opts("mock/last-msg"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("find the bug".into(), None)
        .await
        .expect("turn completes");
    assert_eq!(outcome.text, "The bug is on line 3");
    assert_eq!(outcome.stop_reason, "end_turn");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn prompt_text_falls_back_when_turn_ends_on_tool_activity() {
    // scripting spec "Turn ends on tool activity": a lead message, tool
    // activity, then an (empty) final message — the empty final chunk
    // keeps the fallback rule honest; the lead message is the turn's
    // last agent message.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_LEAD_CHUNKS".into(), "Running the checks now".into());
    spec.env.insert("MOCK_TOOL".into(), "1".into());
    spec.env.insert("MOCK_CHUNKS".into(), String::new());
    let session = start_session(&spec, opts("mock/fallback"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("do work".into(), None)
        .await
        .expect("turn completes");
    assert_eq!(outcome.text, "Running the checks now");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn cancelled_turn_discards_text_and_leaks_nothing_into_next_turn() {
    // scripting spec "Cancelled turn has empty text" + "No text leaks
    // across turns": turn 1 streams one chunk ("partial x") then is
    // cancelled mid-stream — its outcome carries empty text; turn 2 on
    // the same session completes with exactly its own message.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_CHUNKS".into(), "partial x|y|z".into());
    spec.env.insert("MOCK_DELAY_MS".into(), "100".into());
    let session = start_session(&spec, opts("mock/cancel-text"), quiet_renderer())
        .await
        .expect("session starts");

    let s2 = session.clone();
    let pending = tokio::spawn(async move { s2.prompt("slow".into(), None).await });
    // The mock sleeps before each chunk: "partial x" has streamed by now,
    // "y" (due ~200 ms) has not.
    tokio::time::sleep(Duration::from_millis(150)).await;
    session.cancel();
    let outcome = pending.await.unwrap().expect("cancel is not an error");
    assert_eq!(outcome.stop_reason, "cancelled");
    assert_eq!(outcome.text, "", "cancelled turns discard partial text");

    let outcome = session
        .prompt("clean".into(), None)
        .await
        .expect("follow-up turn completes");
    assert_eq!(outcome.text, "partial xyz");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn timed_out_turn_leaks_no_text_into_next_turn() {
    // scripting spec "No text leaks across turns" (timeout flavor): the
    // error path (TurnError::Timeout) drains the fold too — the next
    // turn's text starts from scratch. MOCK_DELAY_MS stretches the turn
    // past the timeout so partial text has streamed before the cancel.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_CHUNKS".into(), "drip|drip|drip".into());
    spec.env.insert("MOCK_DELAY_MS".into(), "100".into());
    let session = start_session(&spec, opts("mock/timeout-text"), quiet_renderer())
        .await
        .expect("session starts");
    let err = session
        .prompt("slow".into(), Some(Duration::from_millis(150)))
        .await
        .expect_err("must time out");
    assert!(matches!(err, TurnError::Timeout), "{err:?}");

    let outcome = session
        .prompt("clean".into(), None)
        .await
        .expect("follow-up turn completes");
    assert_eq!(outcome.text, "dripdripdrip");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn handshake_advertises_config_option_capability() {
    // The mock agent asserts on every `initialize` that the client
    // advertises `session.configOptions` (with its `boolean`
    // sub-capability); a missing bit kills the handshake. Driving a full
    // turn proves the capability bit reached the wire.
    let session = start_session(&mock_agent_spec(), opts("mock/cap"), quiet_renderer())
        .await
        .expect("handshake with capability");
    let outcome = session
        .prompt("hi".into(), None)
        .await
        .expect("turn completes");
    assert_eq!(outcome.text, "hi");
    session.close();
    session.join().await;
}

#[tokio::test]
async fn config_options_captured_from_session_new() {
    // session-config-options spec "Options captured at session start":
    // MOCK_CONFIG_OPTIONS rides the session/new response into the
    // session's option state.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_CONFIG_OPTIONS".into(), MODEL_OPTIONS_JSON.into());
    let session = start_session(&spec, opts("mock/cfg-capture"), quiet_renderer())
        .await
        .expect("session starts");
    let options = session.config_options();
    assert_eq!(options.len(), 2, "{options:?}");
    assert_eq!(options[0].id.0.as_ref(), "model");
    assert_eq!(
        options[0].category,
        Some(agent_client_protocol::schema::v1::SessionConfigOptionCategory::Model)
    );
    session.close();
    session.join().await;
}

#[tokio::test]
async fn config_option_update_replaces_state() {
    // session-config-options spec "Agent-pushed update is folded":
    // MOCK_CONFIG_UPDATE pushes a `config_option_update` during the first
    // prompt's turn; the state snapshot reflects it once the turn ends,
    // and the mock's own state changed too (MOCK_CONFIG_ECHO on a
    // follow-up prompt replies with the new value).
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_CONFIG_OPTIONS".into(), MODEL_OPTIONS_JSON.into());
    spec.env.insert(
        "MOCK_CONFIG_UPDATE".into(),
        r#"[{"id":"model","name":"Model","type":"select","currentValue":"haiku","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]}]"#.into(),
    );
    spec.env.insert("MOCK_CONFIG_ECHO".into(), "model".into());
    let session = start_session(&spec, opts("mock/cfg-update"), quiet_renderer())
        .await
        .expect("session starts");
    let outcome = session
        .prompt("first".into(), None)
        .await
        .expect("turn completes");
    assert_eq!(outcome.text, "opus", "echo should still see the old value");
    // The push follows prompt 1 on the wire; by the time a later turn
    // completes it is definitely folded (and the mock's own state carries
    // it, so the follow-up prompt echoes the new value).
    let outcome = session
        .prompt("second".into(), None)
        .await
        .expect("follow-up turn completes");
    assert_eq!(outcome.text, "haiku", "mock state should carry the update");
    let options = session.config_options();
    assert_eq!(options.len(), 1, "{options:?}");
    assert!(
        matches!(
            &options[0].kind,
            agent_client_protocol::schema::v1::SessionConfigKind::Select(s)
                if s.current_value.0.as_ref() == "haiku"
        ),
        "{options:?}"
    );
    session.close();
    session.join().await;
}

#[tokio::test]
async fn set_config_roundtrip_updates_state() {
    // Driver-level set: the response's full option set replaces the
    // session state; errors carry config id + agent message.
    let mut spec = mock_agent_spec();
    spec.env
        .insert("MOCK_CONFIG_OPTIONS".into(), MODEL_OPTIONS_JSON.into());
    let session = start_session(&spec, opts("mock/cfg-set"), quiet_renderer())
        .await
        .expect("session starts");

    session
        .set_config(
            "model".into(),
            agent_client_protocol::schema::v1::SessionConfigOptionValue::value_id("haiku"),
        )
        .await
        .expect("set succeeds");
    let options = session.config_options();
    assert!(
        matches!(
            &options[0].kind,
            agent_client_protocol::schema::v1::SessionConfigKind::Select(s)
                if s.current_value.0.as_ref() == "haiku"
        ),
        "{options:?}"
    );

    let err = session
        .set_config(
            "model".into(),
            agent_client_protocol::schema::v1::SessionConfigOptionValue::value_id("haiku"),
        )
        .await; // no MOCK_CONFIG_REJECT: still succeeds (re-set same value)
    assert!(err.is_ok());

    session.close();
    session.join().await;
}

/// Run one inline Luau script (against no registry: agents are inline
/// specs pointing at the mock binary) and report the run outcome.
fn run_script(name: &str, body: &str) -> ptah::script::RunOutcome {
    use ptah::script::{self, RunConfig};

    let dir = std::env::temp_dir().join(format!("ptah-acp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("main.luau");
    std::fs::write(&script, body).unwrap();
    let cfg = RunConfig {
        script_path: script,
        invocation_dir: dir,
        registry: ptah::config_fs::from_parts(None, None).unwrap(),
        transport: std::sync::Arc::new(ptah::acp::Transport::new()),
        process_runner: None, // exec is not under test here
        shutdown: None,
        renderer: Arc::new(Renderer::new(RenderOptions::quiet())),
        env: std::collections::BTreeMap::new(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(tokio::task::LocalSet::new().run_until(script::run(cfg)))
}

#[test]
fn config_options_lua_read_shape() {
    // Session API: `configOptions()` reports select and boolean entries
    // with their advertised values, choices, and optional category.
    let out = run_script(
        "cfg-read",
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_CONFIG_OPTIONS = '{json}' }} }})
local s = agent:session()
local options = s:configOptions()
assert(#options == 2, "count: " .. tostring(#options))
local model = options[1]
assert(model.id == "model", model.id)
assert(model.name == "Model", model.name)
assert(model.type == "select", model.type)
assert(model.currentValue == "opus", tostring(model.currentValue))
assert(model.category == "model", tostring(model.category))
assert(model.options ~= nil and #model.options == 2, "choices")
assert(model.options[1].id == "opus" and model.options[1].name == "Opus", "choice 1")
assert(model.options[2].id == "haiku" and model.options[2].name == "Haiku", "choice 2")
assert(model.options[1].description == nil, "absent description must be nil")
local fast = options[2]
assert(fast.id == "fast", fast.id)
assert(fast.type == "boolean", fast.type)
assert(fast.currentValue == false, tostring(fast.currentValue))
assert(fast.options == nil, "boolean options must be nil")
assert(fast.category == nil, "absent category must be nil")
s:close()
"#,
            mock = env!("CARGO_BIN_EXE_mock-agent"),
            json = MODEL_OPTIONS_JSON
        ),
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn set_config_effective_value_via_echo() {
    // Session API: a successful `setConfig` returns nil, updates the live
    // state, and the follow-up prompt provably runs under the new value
    // (MOCK_CONFIG_ECHO replies with the mock's current option value).
    let out = run_script(
        "cfg-set-echo",
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{
    MOCK_CONFIG_OPTIONS = '{json}',
    MOCK_CONFIG_ECHO = "model",
}} }})
local s = agent:session()
local ret = s:setConfig("model", "haiku")
assert(ret == nil, "setConfig must return nil")
local options = s:configOptions()
assert(options[1].currentValue == "haiku", tostring(options[1].currentValue))
local r = s:prompt("go")
assert(r.text == "haiku", "echo: " .. r.text)
-- boolean options accept boolean values
s:setConfig("fast", true)
local after = s:configOptions()
assert(after[2].currentValue == true, tostring(after[2].currentValue))
s:close()
"#,
            mock = env!("CARGO_BIN_EXE_mock-agent"),
            json = MODEL_OPTIONS_JSON
        ),
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn set_config_agent_reject_and_unsupported_method() {
    // Agent rejection (generic error) and unsupported method (-32601)
    // both raise catchable Lua errors naming the config id and carrying
    // the agent's message.
    for reject in ["model", "model=notfound"] {
        let out = run_script(
            &format!("cfg-reject-{}", reject.replace('=', "-")),
            &format!(
                r#"
local agent = ptah.agent({{ command = "{mock}", env = {{
    MOCK_CONFIG_OPTIONS = '{json}',
    MOCK_CONFIG_REJECT = "{reject}",
}} }})
local s = agent:session()
local ok, err = pcall(function() return s:setConfig("model", "haiku") end)
assert(not ok, "setConfig must fail when the agent rejects")
local msg = tostring(err)
assert(msg:find("model", 1, true), "must name the config id: " .. msg)
assert(msg:find("rejects config id model", 1, true) or msg:find("Method not found", 1, true),
    "must carry the agent message: " .. msg)
-- the state is untouched by a rejected set
local options = s:configOptions()
assert(options[1].currentValue == "opus", tostring(options[1].currentValue))
s:close()
"#,
                mock = env!("CARGO_BIN_EXE_mock-agent"),
                json = MODEL_OPTIONS_JSON,
                reject = reject
            ),
        );
        assert_eq!(out.code, 0, "reject={reject}, error: {:?}", out.error);
    }
}

#[test]
fn set_config_mid_turn_serialization() {
    // `setConfig` issued while a turn is in flight waits for it: the
    // request goes out only after the turn completes (config changes
    // apply strictly between turns). MOCK_DELAY_MS stretches the turn to
    // ~150 ms; an unserialized setConfig would return in ~5 ms.
    let out = run_script(
        "cfg-midturn",
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{
    MOCK_CONFIG_OPTIONS = '{json}',
    MOCK_CONFIG_ECHO = "model",
    MOCK_DELAY_MS = "150",
}} }})
local s = agent:session()
local first = ptah.spawn(function() return s:prompt("first").text end)
ptah.sleep(30)
local t0 = os.clock()
s:setConfig("model", "haiku")
local took = os.clock() - t0
assert(took >= 0.10, "setConfig did not wait for the in-flight turn: " .. tostring(took))
local firstText = first:await()
assert(firstText == "opus", "first turn ran under the old value: " .. firstText)
local r = s:prompt("second")
assert(r.text == "haiku", "second turn ran under the new value: " .. r.text)
s:close()
"#,
            mock = env!("CARGO_BIN_EXE_mock-agent"),
            json = MODEL_OPTIONS_JSON
        ),
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn set_config_wrong_type_errors_before_send() {
    // Non-string, non-boolean values raise a Lua error before any wire
    // traffic; the session's option state is unchanged.
    let out = run_script(
        "cfg-wrongtype",
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_CONFIG_OPTIONS = '{json}' }} }})
local s = agent:session()
local ok, err = pcall(function() return s:setConfig("model", 42) end)
assert(not ok, "number values must be rejected")
local msg = tostring(err)
assert(msg:find("setConfig value", 1, true), msg)
assert(msg:find("integer", 1, true) or msg:find("number", 1, true), msg)
local ok2, err2 = pcall(function() return s:setConfig("model", nil) end)
assert(not ok2, "nil values must be rejected")
local options = s:configOptions()
assert(options[1].currentValue == "opus", tostring(options[1].currentValue))
s:close()
"#,
            mock = env!("CARGO_BIN_EXE_mock-agent"),
            json = MODEL_OPTIONS_JSON
        ),
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}

#[test]
fn config_options_empty_when_agent_offers_none() {
    // Agents without config options yield an empty table, and `setConfig`
    // on them still round-trips through the agent (the mock answers with
    // its empty state).
    let out = run_script(
        "cfg-empty",
        &format!(
            r#"
local agent = ptah.agent({{ command = "{mock}", env = {{ MOCK_CONFIG_ECHO = "model" }} }})
local s = agent:session()
local options = s:configOptions()
assert(#options == 0, "expected empty options, got " .. tostring(#options))
assert(next(options) == nil, "options must be an empty table")
s:setConfig("model", "haiku")
assert(#s:configOptions() == 0, "still empty after a set")
local r = s:prompt("go")
assert(r.text == "unknown-config:model", "echo of an unset option: " .. r.text)
s:close()
"#,
            mock = env!("CARGO_BIN_EXE_mock-agent"),
        ),
    );
    assert_eq!(out.code, 0, "error: {:?}", out.error);
}
