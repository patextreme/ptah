//! Per-run runtime state, plus the run's configuration and result types.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use ptah_core::config::Registry;
use ptah_core::ports::{AgentTransport, EventSink, InteractionMode, ProcessRunner};
use ptah_core::session::SessionHandle;
use ptah_core::task::TaskRegistry;

/// Reserved sink pseudo-label for script-level (non-session) events —
/// today the `ptah.exec` lifecycle lines. Not a legal user session id
/// (rejected at session-options validation), so a render can never be
/// confused about attribution.
pub(crate) const EXEC_LABEL: &str = "exec";

/// Bookkeeping for one in-flight `ptah.exec`: the teardown signal.
/// `run`'s teardown marks `killed` and wakes `cancel` so the awaiting
/// binding drops its port future — and the runner's cancel-safety
/// contract (dropping the future kills the process group) does the
/// rest. The flag exists because `notify_waiters` is lost on tasks
/// that have not reached their `notified()` await yet: a binding that
/// registers after the signal still observes `killed` (there is no
/// yield between its check and arming the waiter, so no interleave is
/// possible on the single-threaded runtime).
#[derive(Default)]
pub(crate) struct ExecEntry {
    pub(crate) killed: std::cell::Cell<bool>,
    pub(crate) cancel: tokio::sync::Notify,
}

/// Everything the Lua environment needs, shared per run (single-threaded).
pub(crate) struct RuntimeState {
    pub(crate) registry: Registry,
    pub(crate) sink: Arc<dyn EventSink>,
    /// Where agent sessions come from (the ACP stdio transport by
    /// Injected via [`RunConfig::transport`] (the CLI composes the ACP
    /// stdio adapter there).
    pub(crate) transport: Arc<dyn AgentTransport>,
    /// The injected process runner funding `ptah.exec` (`None` in
    /// embedders that inject no capability — the binding then raises a
    /// clear runtime error instead of touching the world).
    pub(crate) process_runner: Option<Arc<dyn ProcessRunner>>,
    /// The resolved interaction posture funding `ptah.ask`: an injected
    /// provider, the `none` posture, or nothing resolved (both
    /// non-provider states raise distinct errors at call time).
    pub(crate) interaction: InteractionMode,
    /// The entry script's path — ask attribution labels carry its
    /// basename (`ask {n} {script}`).
    pub(crate) script_path: PathBuf,
    pub(crate) invocation_dir: PathBuf,
    pub(crate) tasks: Rc<TaskRegistry>,
    pub(crate) sessions: RefCell<Vec<SessionHandle>>,
    /// In-flight `ptah.exec` calls, registered at start and removed at
    /// end; teardown drains this to kill live process groups.
    pub(crate) execs: RefCell<Vec<Rc<ExecEntry>>>,
    /// Per-run ask counter (monotonic from 1) for attribution labels.
    pub(crate) ask_counter: Cell<u64>,
    /// Serializes concurrent `ptah.ask` calls in FIFO issue order: the
    /// lock is taken before an ask emits anything, so every provider
    /// (including test fakes) sees exactly one ask at a time and prompts
    /// never interleave. Serialization lives here, in the binding, not
    /// in the port — a future provider that could safely parallelize
    /// would need a spec change, not just an impl.
    pub(crate) ask_lock: tokio::sync::Mutex<()>,
    /// Snapshot of ptah's environment for `os.getenv`: captured once at
    /// the composition root and injected — the binding performs no
    /// ambient environment read.
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) exit_code: Cell<Option<i32>>,
}

/// Configuration for one `ptah run`.
pub struct RunConfig {
    pub script_path: PathBuf,
    pub invocation_dir: PathBuf,
    pub registry: Registry,
    /// Where agent sessions come from: the injected ACP stdio transport
    /// (composed in the CLI crate — the adapter choice is a composition
    /// decision, not a scripting-runtime one).
    pub transport: Arc<dyn AgentTransport>,
    /// The injected process runner for `ptah.exec` (tokio `/bin/sh -c`
    /// implementation composed in the CLI crate). `None` leaves
    /// `ptah.exec` raising a clear "no runner injected" error — the
    /// scripting runtime stays free of ambient subprocess powers.
    pub process_runner: Option<Arc<dyn ProcessRunner>>,
    /// The resolved interaction posture for `ptah.ask`, resolved once at
    /// the composition root (`--ask` > `PTAH_ASK` > project `[ask]` >
    /// user `[ask]` > auto-detect) and injected here. `Unresolved` (the
    /// default for embedders and tests that inject nothing) makes
    /// `ptah.ask` raise the no-provider error at call time; `Prohibited`
    /// raises the prohibited error.
    pub interaction: InteractionMode,
    /// Outer cancellation for the run: fired by the embedding process
    /// when a termination signal arrives (the composition root forwards
    /// SIGINT/SIGTERM, the value carrying the exit code to report —
    /// 130/143 by shell convention). On fire the run loop abandons the
    /// script and rides the teardown path; the carried code becomes the
    /// run's outcome. `None` — no external cancel (tests, embedders);
    /// the run simply ends however the script ends. Control-plane data,
    /// deliberately not a port: nothing here touches the world, it only
    /// asks the run to stop.
    pub shutdown: Option<tokio::sync::watch::Receiver<i32>>,
    /// The output sink: `Arc<Renderer>` coerces here at construction
    /// sites, so callers keep building the terminal renderer.
    pub renderer: Arc<dyn EventSink>,
    /// Snapshot of ptah's environment for `os.getenv`, captured once at
    /// the composition root (`env::vars_os()` filtered to valid UTF-8,
    /// so non-UTF-8 entries read as unset — `env::vars()` panics on
    /// them). Injection, not an ambient read: the scripting runtime
    /// never touches `std::env`. Tests default to an empty snapshot.
    pub env: BTreeMap<String, String>,
}

/// Result of one run.
#[derive(Debug, Default)]
pub struct RunOutcome {
    pub code: i32,
    /// Uncaught script error (printed to stderr by the CLI).
    pub error: Option<String>,
    /// Task errors never delivered to the script (printed to stderr; run fails).
    pub undelivered_errors: Vec<String>,
}

pub(crate) fn runtime_state(lua: &mlua::Lua) -> mlua::Result<Rc<RuntimeState>> {
    lua.app_data_ref::<Rc<RuntimeState>>()
        .map(|s| Rc::clone(&s))
        .ok_or_else(|| mlua::Error::runtime("ptah runtime state missing"))
}
