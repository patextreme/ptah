//! Ports and policies: the seams where adapters plug into the core.
//!
//! Six funded ports live here: [`AgentTransport`], [`ConfigSource`],
//! [`EventSink`], [`InteractionPolicy`], [`ProcessRunner`] (funding
//! `ptah.exec`), and [`AskProvider`] (funding `ptah.ask`), plus the
//! [`InteractionMode`] the composition root resolves once per run.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    PermissionOption, PermissionOptionId, PermissionOptionKind,
};

use crate::config::{AgentSpec, ConfigError, Registry};
use crate::events::SessionEvent;
use crate::session::{SessionError, SessionHandle, SessionOptions};

/// Wiring for the injected typed-results MCP server (the `ptah __bridge`
/// subprocess suggested to agents in `session/new { mcpServers }`).
///
/// The value is data, not an import: the driver injects the server by
/// these names, and the bridge binary reads the same env vars — a unit
/// test in `src/bridge.rs` pins the two definitions together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeConfig {
    /// Server name agents see (they derive `mcp__<name>__result_submit`).
    pub server_name: &'static str,
    /// Env var carrying the session's result socket path.
    pub addr_env: &'static str,
    /// Env var carrying the declared JSON schema.
    pub schema_env: &'static str,
}

impl BridgeConfig {
    /// The ptah bridge binary's wiring.
    pub const fn ptah_bridge() -> Self {
        Self {
            server_name: "ptah",
            addr_env: "PTAH_BRIDGE_ADDR",
            schema_env: "PTAH_RESULT_SCHEMA",
        }
    }
}

/// How the runtime starts agent sessions. The ACP stdio adapter
/// implements it; the port is shaped by what the script layer consumes —
/// spawn a session, then `prompt`/`cancel`/`close` and config options on
/// the returned [`SessionHandle`] — which is exactly what mocks and
/// future transports must satisfy. Manually boxed futures keep the trait
/// object-safe without an async-trait dependency.
pub trait AgentTransport: Send + Sync {
    /// Start one agent session and drive it until closed. The returned
    /// handle's `session_id` carries the agent-assigned ACP session id
    /// (stable for the session's lifetime); transports must supply one.
    fn start_session<'a>(
        &'a self,
        spec: &'a AgentSpec,
        opts: SessionOptions,
        sink: Arc<dyn EventSink>,
    ) -> Pin<Box<dyn Future<Output = Result<SessionHandle, SessionError>> + 'a>>;
}

/// Where the agent registry comes from. The TOML/fs loader implements it
/// today (user + project layers, project-wins precedence); the port is
/// the seam for other sources (embedded, remote) without touching
/// callers.
pub trait ConfigSource: Send + Sync {
    /// Discover and load the registry for an invocation directory.
    fn discover(&self, invocation_dir: &Path) -> Result<Registry, ConfigError>;
}

/// Why an execution could not run at all: data for the scripting layer
/// to raise, not an I/O type of core's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The command could not be spawned (the message names the command
    /// and the OS error).
    Spawn(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Spawn(msg) => write!(f, "could not run command {msg}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Outcome of one process execution: captured output plus how it ended.
/// `exit_code` is `None` only when no exit status exists — the command
/// timed out (`timed_out: true`, the group already killed) or died
/// without reporting a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    /// The command's exit code, when one exists. A signal death is
    /// normalized to the shell's `128 + signal` convention by the
    /// runner, so `None` + `timed_out: false` means "never ran to a
    /// status" (a spawn failure raises [`ExecError`] instead).
    pub exit_code: Option<i32>,
    /// Everything the command wrote to stdout (captured, not streamed).
    pub stdout: String,
    /// Everything the command wrote to stderr.
    pub stderr: String,
    /// `timeoutMs` elapsed: the process group was killed before this
    /// outcome was returned.
    pub timed_out: bool,
}

/// Where process execution comes from — the fifth port, funding
/// `ptah.exec`. Like the other world-touching seams ([`AgentTransport`],
/// [`ConfigSource`]), the trait is pure data-in/data-out (a command
/// string, an optional timeout budget, captured output back); the tokio
/// `/bin/sh -c` implementation lives at the composition root and is
/// injected through `RunConfig`. A consumer that injects no runner gets
/// a clean "no runner injected" error from the binding instead of an
/// ambient shell — "who may touch the world" stays a composition
/// decision. Dropping the returned future before it resolves is the
/// cancellation contract: implementations must kill the command's
/// whole process group (no orphans outlive the run).
pub trait ProcessRunner: Send + Sync {
    /// Run `cmd` under `/bin/sh -c`, capturing stdout/stderr. `timeout_ms`
    /// of `None` means no budget. Manually boxed futures keep the trait
    /// object-safe, mirroring [`AgentTransport::start_session`].
    fn run<'a>(
        &'a self,
        cmd: &'a str,
        timeout_ms: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<ExecOutcome, ExecError>> + Send + 'a>>;
}

/// One question from a script to a human, everything a provider needs
/// to deliver it. `attribution` is the per-run ask label
/// (`ask {n} {script_basename}`) so a provider can name which run and
/// script is asking (the renderer consumes it as the sink label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskRequest {
    /// The question, rendered as the prompt line.
    pub prompt: String,
    /// Optional context, rendered as one indented line under the prompt.
    pub details: Option<String>,
    /// `ask {n} {script_basename}` — which ask of which run is asking.
    pub attribution: String,
}

/// A provider's answer: response-or-abort, never an error (abort is a
/// normal result the workflow handles like any other value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    /// The human answered; `text` is the answer as received, unprocessed
    /// (v1: one line).
    Respond { text: String },
    /// The human chose the provider's abort gesture.
    Abort,
}

/// Why a provider could not produce an answer. Data for the binding to
/// raise: `InputClosed` and `Failed(String)` map to distinct, stable
/// error messages; the provider never invents messages of its own for
/// these two conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The input source closed with no gesture (stdin EOF on a
    /// non-terminal) — the end-of-input condition, distinct from abort.
    InputClosed,
    /// The provider itself failed (the string names the failure).
    Failed(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::InputClosed => write!(f, "ask input closed with no answer"),
            AskError::Failed(msg) => write!(f, "ask provider failed: {msg}"),
        }
    }
}

impl std::error::Error for AskError {}

/// Where human answers to `ptah.ask` come from — the sixth port. Pure
/// data-in (one [`AskRequest`]) / data-out ([`AskOutcome`] or
/// [`AskError`]); the concrete stdin implementation (and any future
/// webhook/Slack/TUI provider) lives at the composition root and is
/// injected through `RunConfig`, like [`ProcessRunner`]. Dropping the
/// returned future before it resolves is the cancellation contract: the
/// pending ask simply ends with the run (no abort is delivered — abort
/// is a human answer, cancellation is process-level).
pub trait AskProvider: Send + Sync {
    /// Deliver one ask and resolve with the human's answer. Manually
    /// boxed futures keep the trait object-safe, mirroring
    /// [`AgentTransport::start_session`].
    fn ask<'a>(
        &'a self,
        request: AskRequest,
    ) -> Pin<Box<dyn Future<Output = Result<AskOutcome, AskError>> + Send + 'a>>;
}

/// The run's interaction posture, resolved exactly once at the
/// composition root (`--ask` > `PTAH_ASK` > project `[ask]` > user
/// `[ask]` > auto-detect) and consumed by the `ptah.ask` binding, the
/// run pre-flight, and `ptah check` alike. `Prohibited` is the
/// deliberate `none` posture; `Unresolved` means nothing selected a
/// provider and auto-detection could not (no terminal) — a
/// configuration gap, deliberately not folded into `Prohibited` so the
/// two error messages never lie about which problem the operator has.
pub enum InteractionMode {
    /// A provider resolved; asks deliver through it.
    Provider(Arc<dyn AskProvider>),
    /// `none` was selected: `ptah.ask` raises prohibited.
    Prohibited,
    /// Nothing resolved a provider and auto-detect could not apply.
    Unresolved,
}

impl Clone for InteractionMode {
    fn clone(&self) -> Self {
        match self {
            InteractionMode::Provider(p) => InteractionMode::Provider(Arc::clone(p)),
            InteractionMode::Prohibited => InteractionMode::Prohibited,
            InteractionMode::Unresolved => InteractionMode::Unresolved,
        }
    }
}

impl std::fmt::Debug for InteractionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The provider object is opaque; the variant is the diagnostic.
        match self {
            InteractionMode::Provider(_) => f.write_str("Provider(<injected>)"),
            InteractionMode::Prohibited => f.write_str("Prohibited"),
            InteractionMode::Unresolved => f.write_str("Unresolved"),
        }
    }
}

/// Where session events go. The session driver folds wire updates and
/// emits structured [`SessionEvent`]s through this port; the terminal
/// renderer implements it today, and a TUI or structured logger can take
/// the same seam without touching the driver.
pub trait EventSink: Send + Sync {
    /// One structured event, attributed to a session by its label.
    fn emit(&self, label: &str, event: SessionEvent);
    /// A script-initiated log line (`ptah.log`): not a session event
    /// and not suppressed by `--quiet`.
    fn script_log(&self, message: &str);
}

/// Decisions for agent→client requests that would otherwise need a user
/// present. ptah runs headless; the policy is the exact seam a TUI (or
/// any interactive front end) needs to make permissions interactive
/// without touching transport code.
pub trait InteractionPolicy: Send + Sync {
    /// The option id to answer `session/request_permission` with, or
    /// `None` when the offer has no allow option to select (the adapter
    /// then answers method-not-found).
    fn select_permission(&self, options: &[PermissionOption]) -> Option<PermissionOptionId>;
}

/// The headless posture: prefer `AllowAlways`, else the first other
/// allow-kind option (documented in the README; choosing `AllowAlways`
/// may let the agent persist an allow rule in its own settings beyond
/// the run).
pub struct HeadlessPolicy;

impl InteractionPolicy for HeadlessPolicy {
    fn select_permission(&self, options: &[PermissionOption]) -> Option<PermissionOptionId> {
        select_allow_option(options)
    }
}

/// Pick the option to answer a permission request with: the first
/// `AllowAlways` when offered, otherwise the first other allow-kind
/// option. `None` when the offer has no allow option at all.
pub(crate) fn select_allow_option(options: &[PermissionOption]) -> Option<PermissionOptionId> {
    options
        .iter()
        .find(|o| matches!(o.kind, PermissionOptionKind::AllowAlways))
        .or_else(|| {
            options
                .iter()
                .find(|o| matches!(o.kind, PermissionOptionKind::AllowOnce))
        })
        .map(|o| o.option_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, kind: PermissionOptionKind) -> PermissionOption {
        PermissionOption::new(id.to_string(), "label", kind)
    }

    #[test]
    fn allow_selection_prefers_allow_always() {
        let options = vec![
            option("allow_once", PermissionOptionKind::AllowOnce),
            option("allow_always", PermissionOptionKind::AllowAlways),
        ];
        assert_eq!(
            select_allow_option(&options),
            Some(PermissionOptionId::new("allow_always"))
        );
    }

    #[test]
    fn allow_selection_falls_back_to_any_allow_kind() {
        let options = vec![
            option("reject_once", PermissionOptionKind::RejectOnce),
            option("allow_once", PermissionOptionKind::AllowOnce),
        ];
        assert_eq!(
            select_allow_option(&options),
            Some(PermissionOptionId::new("allow_once"))
        );
    }

    #[test]
    fn allow_selection_reject_only_offer_gets_method_not_found() {
        let options = vec![
            option("reject_once", PermissionOptionKind::RejectOnce),
            option("reject_always", PermissionOptionKind::RejectAlways),
        ];
        assert_eq!(select_allow_option(&options), None);
        assert_eq!(select_allow_option(&[]), None);
    }

    #[test]
    fn headless_policy_implements_the_selection_rule() {
        let policy = HeadlessPolicy;
        let options = vec![
            option("reject_once", PermissionOptionKind::RejectOnce),
            option("allow_once", PermissionOptionKind::AllowOnce),
            option("allow_always", PermissionOptionKind::AllowAlways),
        ];
        let dyn_policy: &dyn InteractionPolicy = &policy;
        assert_eq!(
            dyn_policy.select_permission(&options),
            Some(PermissionOptionId::new("allow_always"))
        );
        assert_eq!(dyn_policy.select_permission(&[]), None);
    }
}
