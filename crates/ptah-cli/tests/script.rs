//! Unit tests for the sandboxed Luau environment, require resolution, and
//! the task runtime primitives (with fake async ops).

use std::sync::Arc;

use mlua::FromLua as _;
use mlua::luau::Require as _;
use mlua::{Function, Lua, Value};

use ptah::render::{RenderOptions, Renderer};
use ptah::script::{self, RunConfig, require::ScriptRequirer};
use ptah::task::{TaskRegistry, spawn};
use ptah_core::ports::InteractionMode;

/// Build a Lua env exactly like production (sandbox + ptah table).
fn test_lua(script_dir: &std::path::Path) -> Lua {
    let cfg = RunConfig {
        script_path: script_dir.join("main.luau"),
        invocation_dir: script_dir.to_path_buf(),
        registry: ptah::config_fs::from_parts(None, None).unwrap(),
        transport: std::sync::Arc::new(ptah::acp::Transport::new()),
        process_runner: None,
        interaction: InteractionMode::Unresolved,
        shutdown: None,
        renderer: Arc::new(Renderer::new(RenderOptions::quiet())),
        env: std::collections::BTreeMap::new(),
    };
    script::setup_lua(&cfg).unwrap()
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ptah-script-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// 6.1 sandbox
// ---------------------------------------------------------------------------

#[test]
fn sandboxed_globals_absent() {
    let dir = tmpdir("sandbox");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);
    let globals = lua.globals();

    for name in ["io", "debug", "package"] {
        let v: Value = globals.get(name).unwrap();
        assert!(matches!(v, Value::Nil), "{name} must be absent, got {v:?}");
    }

    // coroutine is restricted to `yield` (required by the embedded async
    // runtime); the scheduling functions are absent.
    let co: mlua::Table = globals.get("coroutine").unwrap();
    for name in ["create", "resume", "wrap", "status", "running"] {
        let v: Value = co.get(name).unwrap();
        assert!(matches!(v, Value::Nil), "coroutine.{name} must be absent");
    }
    assert!(co.get::<Function>("yield").is_ok());

    // raw-global escapes are poisoned: they raise on call
    for expr in ["loadstring('return 1')", "collectgarbage('count')"] {
        let err = lua.load(expr).eval::<()>().unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    // os restricted to time/clock/getenv
    let os: mlua::Table = globals.get("os").unwrap();
    assert!(os.get::<Function>("time").is_ok());
    assert!(os.get::<Function>("clock").is_ok());
    assert!(os.get::<Function>("getenv").is_ok());
    for name in ["execute", "setenv", "remove", "rename", "exit", "date"] {
        let v: Value = os.get(name).unwrap();
        assert!(matches!(v, Value::Nil), "os.{name} must be absent");
    }
}

#[test]
fn print_passthrough_writes_stdout() {
    let dir = tmpdir("print");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);
    // print exists and callable; output goes to real stdout (assert via no-panic).
    let _: () = lua.load("print('hello from script')").eval().unwrap();
}

#[test]
fn curated_stdlib_present() {
    let dir = tmpdir("stdlib");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);
    for lib in ["string", "table", "math", "utf8", "bit32", "buffer"] {
        let v: Value = lua.globals().get(lib).unwrap();
        assert!(!matches!(v, Value::Nil), "{lib} must exist");
    }
}

// ---------------------------------------------------------------------------
// 6.2 require
// ---------------------------------------------------------------------------

#[test]
fn require_sibling_missing_and_cached() {
    let dir = tmpdir("require");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("main.luau"), "").unwrap();
    std::fs::write(dir.join("lib/util.luau"), "return { n = 41 + 1 }").unwrap();
    let lua = test_lua(&dir);
    let entry = format!("@{}/main.luau", dir.display());

    // sibling module loads
    let v: u32 = lua
        .load("local m = require('./lib/util') return m.n")
        .set_name(entry.clone())
        .eval()
        .unwrap();
    assert_eq!(v, 42);

    // caching: same require path returns the same module table
    let same: bool = lua
        .load("return require('./lib/util') == require('./lib/util')")
        .set_name(entry.clone())
        .eval()
        .unwrap();
    assert!(same, "second require must return the cached module");

    // missing module raises an error naming the unresolved path
    let err = lua
        .load("require('./lib/nope')")
        .set_name(entry.clone())
        .eval::<()>()
        .unwrap_err();
    assert!(
        err.to_string().contains("nope"),
        "missing-module error must name the path: {err}"
    );

    // cross-tree require: a module outside the entry directory loads
    let shared_name = format!("{}-shared", dir.file_name().unwrap().to_str().unwrap());
    let shared = dir.parent().unwrap().join(&shared_name);
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("helper.luau"), "return { n = 7 }").unwrap();
    let v: u32 = lua
        .load(format!(
            "local m = require('../{shared_name}/helper') return m.n"
        ))
        .set_name(entry.clone())
        .eval()
        .unwrap();
    assert_eq!(v, 7);
    let _ = std::fs::remove_dir_all(&shared);

    // absolute path rejected
    let err = lua
        .load("require('/etc/passwd')")
        .set_name(entry)
        .eval::<()>()
        .unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn requirer_unit_navigation() {
    // direct ScriptRequirer unit checks (also covered in require.rs tests)
    let dir = tmpdir("requirer");
    std::fs::write(dir.join("a.luau"), "").unwrap();
    let mut req = ScriptRequirer::new(dir.clone());
    req.reset(&format!("@{}/a.luau", dir.display())).unwrap();
    assert!(req.has_module()); // a.luau itself resolves
}

// ---------------------------------------------------------------------------
// 6.3 task runtime (fake async ops)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_runtime_semantics() {
    let dir = tmpdir("tasks");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);
    let registry = TaskRegistry::default();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mk = |ms: u64| -> Function {
                lua.load(format!(
                    "return function() ptah.sleep({ms}); return {ms} end"
                ))
                .eval()
                .unwrap()
            };
            let t1 = spawn(&lua, &registry, mk(30)).unwrap();
            let t2 = spawn(&lua, &registry, mk(10)).unwrap();
            let t3 = spawn(&lua, &registry, mk(20)).unwrap();

            let first = |m: mlua::MultiValue, lua: &Lua| -> u64 {
                u64::from_lua(m.into_iter().next().unwrap(), lua).unwrap()
            };
            let v1 = first(t1.await_result().await.unwrap(), &lua);
            let v2 = first(t2.await_result().await.unwrap(), &lua);
            let v3 = first(t3.await_result().await.unwrap(), &lua);
            assert_eq!((v1, v2, v3), (30, 10, 20));

            // await re-raises the task error
            let boom: Function = lua
                .load("return function() error('boom', 0) end")
                .eval()
                .unwrap();
            let tb = spawn(&lua, &registry, boom).unwrap();
            let err = tb.await_result().await.unwrap_err();
            assert!(err.to_string().contains("boom"), "{err}");
        })
        .await;
}

#[test]
fn spawn_from_lua_chunk() {
    let dir = tmpdir("spawn-lua");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    rt.block_on(local.run_until(async {
        let t: String = lua
            .load("local t = ptah.spawn(function() return 7 end) return type(t)")
            .set_name(format!("@{}/main.luau", dir.display()))
            .eval()
            .unwrap();
        assert_eq!(t, "table");
    }));
}

#[test]
fn namespace_map_rename_is_hard() {
    // Scripting spec (revise-script-api): `ptah.map` → `ptah.parallel`
    // with no alias — the old key reads as nil, so a legacy call errors
    // at the call site instead of silently doing something else.
    let dir = tmpdir("parallel-rename");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);

    let gone: Value = lua.load("return ptah.map").eval().unwrap();
    assert!(matches!(gone, Value::Nil), "ptah.map must read as nil");

    let present: Value = lua.load("return ptah.parallel").eval().unwrap();
    assert!(
        matches!(present, Value::Function(_)),
        "ptah.parallel must exist"
    );
    let err = lua
        .load("ptah.map({1}, function() end)")
        .eval::<()>()
        .unwrap_err();
    assert!(
        err.to_string().contains("nil value"),
        "calling the removed name must error: {err}"
    );
}

// ---------------------------------------------------------------------------
// Session id `exec` reservation (shell-exec capability: the pseudo-label)
// ---------------------------------------------------------------------------

#[test]
fn session_id_exec_is_reserved() {
    // The sink attributes `ptah.exec` lifecycle events under the
    // pseudo-label "exec"; a user session with that id would collide, so
    // session-options validation rejects it pre-spawn (the command never
    // runs — the error must not name the agent command).
    let dir = tmpdir("exec-id");
    std::fs::write(dir.join("main.luau"), "").unwrap();
    let lua = test_lua(&dir);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(tokio::task::LocalSet::new().run_until(async {
        let err = lua
            .load(
                r#"
local agent = ptah.agent({ command = "/nonexistent/ptah-test-agent" })
local ok, err = pcall(function()
    return agent:session({ id = "exec" })
end)
assert(not ok, "id `exec` must be rejected")
error(tostring(err), 0)
"#,
            )
            .set_name(format!("@{}/main.luau", dir.display()))
            .eval_async::<()>()
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("reserved"), "{msg}");
        assert!(
            !msg.contains("/nonexistent"),
            "must fail before spawning: {msg}"
        );
    }));
}
