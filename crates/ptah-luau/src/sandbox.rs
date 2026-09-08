//! Sandbox setup: create the sandboxed Luau environment for a run —
//! curated stdlib, the custom `require`, the ptah runtime state, and the
//! `ptah` namespace — with the documented `coroutine` deviation and the
//! poison globals.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use mlua::{Function, Lua, LuaOptions, StdLib, Table};

use ptah_core::task::TaskRegistry;

use crate::require::ScriptRequirer;

use super::bindings::bind_ptah;
use super::state::{RunConfig, RuntimeState};

/// Create the sandboxed Luau environment for a run.
pub fn setup_lua(cfg: &RunConfig) -> mlua::Result<Lua> {
    let lua = Lua::new_with(
        StdLib::TABLE
            | StdLib::OS
            | StdLib::STRING
            | StdLib::UTF8
            | StdLib::BIT
            | StdLib::BUFFER
            | StdLib::MATH,
        LuaOptions::default(),
    )?;
    lua.sandbox(true)?;

    let globals = lua.globals();

    // os: keep only time, clock, and getenv.
    let os = globals.get::<Table>("os")?;
    let os_safe = lua.create_table()?;
    os_safe.set("time", os.get::<Function>("time")?)?;
    os_safe.set("clock", os.get::<Function>("clock")?)?;
    globals.set("os", os_safe)?;

    // require: relative to the requiring file, with no boundary. Canonicalize
    // the entry path so the requirer root lives in the same absolute
    // namespace as chunk names (`@/abs/...`, set in `run`): a relative root
    // would mis-resolve every require made by a script invoked through a
    // relative path (e.g. `ptah run dir/s.luau`).
    let entry = std::fs::canonicalize(&cfg.script_path).unwrap_or_else(|_| cfg.script_path.clone());
    let script_root = entry
        .parent()
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
    let require_fn = lua.create_require_function(ScriptRequirer::new(script_root))?;
    globals.set("require", require_fn)?;

    let state = Rc::new(RuntimeState {
        registry: cfg.registry.clone(),
        sink: cfg.renderer.clone(),
        transport: cfg.transport.clone(),
        process_runner: cfg.process_runner.clone(),
        invocation_dir: cfg.invocation_dir.clone(),
        tasks: Rc::new(TaskRegistry::default()),
        sessions: RefCell::new(Vec::new()),
        execs: RefCell::new(Vec::new()),
        env: cfg.env.clone(),
        exit_code: Cell::new(None),
    });
    lua.set_app_data(state.clone());

    // os.getenv: read-only window onto the injected startup snapshot
    // (standard Luau contract — the value, nil when unset, `""` when
    // set to the empty string). One string argument, no enumeration,
    // no mutation surface: `os.setenv` does not exist. Bound through
    // RuntimeState — the snapshot is the injected capability, not an
    // ambient read (there is no `std::env` on this path).
    {
        let env = state.env.clone();
        let os: Table = globals.get("os")?;
        os.set(
            "getenv",
            lua.create_function(move |_, name: mlua::Value| match name {
                mlua::Value::String(s) => {
                    let name = s.to_str()?;
                    Ok(env.get(&*name).cloned())
                }
                _ => Err(mlua::Error::runtime(
                    "os.getenv expects a variable name (string)",
                )),
            })?,
        )?;
    }

    bind_ptah(&lua)?;

    // mlua's async machinery re-reads the global `coroutine` whenever an
    // async callback is created (some of ours are created lazily at
    // runtime), so the global must remain a table with a real `yield`.
    // Restrict it to exactly that: no create/resume/wrap (concurrency is
    // ptah.spawn's job). Documented deviation from the curated-stdlib list.
    let globals = lua.globals();
    let coroutine = globals.get::<Table>("coroutine")?;
    let coroutine_safe = lua.create_table()?;
    coroutine_safe.set("yield", coroutine.get::<Function>("yield")?)?;
    globals.set("coroutine", coroutine_safe)?;
    let poison = lua.create_function(|_lua, ()| -> mlua::Result<()> {
        Err(mlua::Error::runtime(
            "this global is not available in ptah scripts",
        ))
    })?;
    globals.set("loadstring", poison.clone())?;
    globals.set("collectgarbage", poison)?;

    Ok(lua)
}

#[cfg(test)]
mod tests {
    //! The `os.getenv` contract at the binding level: reads observe the
    //! injected snapshot only (set/unset/empty, argument shape, no
    //! mutation surface, stable repeated reads).

    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use ptah_core::config::{AgentSpec, Registry};
    use ptah_core::events::SessionEvent;
    use ptah_core::ports::{AgentTransport, EventSink};
    use ptah_core::session::{SessionError, SessionHandle, SessionOptions};

    use super::*;

    /// A transport that never starts a session — these tests never
    /// touch agents, so the double only has to compile.
    struct NoTransport;

    impl AgentTransport for NoTransport {
        fn start_session<'a>(
            &'a self,
            _spec: &'a AgentSpec,
            _opts: SessionOptions,
            _sink: Arc<dyn EventSink>,
        ) -> Pin<Box<dyn Future<Output = Result<SessionHandle, SessionError>> + 'a>> {
            Box::pin(async { Err(SessionError::Handshake("no transport in tests".into())) })
        }
    }

    struct NullSink;

    impl EventSink for NullSink {
        fn emit(&self, _label: &str, _event: SessionEvent) {}
        fn script_log(&self, _message: &str) {}
    }

    fn lua_with_env(env: &[(&str, &str)]) -> Lua {
        let cfg = RunConfig {
            script_path: PathBuf::from("/nonexistent/main.luau"),
            invocation_dir: PathBuf::from("."),
            registry: Registry::default(),
            transport: Arc::new(NoTransport),
            process_runner: None,
            shutdown: None,
            renderer: Arc::new(NullSink),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect::<BTreeMap<_, _>>(),
        };
        setup_lua(&cfg).unwrap()
    }

    #[test]
    fn getenv_reads_the_snapshot() {
        let lua = lua_with_env(&[("PTAH_ENV_PROBE", "x"), ("PTAH_ENV_EMPTY", "")]);
        let v: Option<String> = lua
            .load(r#"return os.getenv("PTAH_ENV_PROBE")"#)
            .eval()
            .unwrap();
        assert_eq!(v.as_deref(), Some("x"), "set variable returns its value");
        // Set-to-empty is "", distinct from unset.
        let e: String = lua
            .load(r#"return os.getenv("PTAH_ENV_EMPTY")"#)
            .eval()
            .unwrap();
        assert_eq!(e, "", "set-to-empty returns the empty string");
        // Unset reads as nil.
        let n: Option<String> = lua
            .load(r#"return os.getenv("PTAH_ENV_ABSENT")"#)
            .eval()
            .unwrap();
        assert!(n.is_none(), "unset variable reads as nil");
        // Repeated reads are stable — same snapshot, no side effects.
        let again: Option<String> = lua
            .load(r#"return os.getenv("PTAH_ENV_PROBE")"#)
            .eval()
            .unwrap();
        assert_eq!(again.as_deref(), Some("x"), "repeated read is stable");
    }

    #[test]
    fn getenv_argument_must_be_a_string() {
        let lua = lua_with_env(&[]);
        for expr in ["os.getenv()", "os.getenv(42)"] {
            let err = lua.load(expr).eval::<Option<String>>().expect_err(expr);
            assert!(!err.to_string().is_empty(), "{expr} must raise");
        }
    }

    #[test]
    fn no_environment_mutation_surface() {
        let lua = lua_with_env(&[]);
        let os: Table = lua.globals().get("os").unwrap();
        let v: mlua::Value = os.get("setenv").unwrap();
        assert!(matches!(v, mlua::Value::Nil), "os.setenv must be absent");
    }
}
