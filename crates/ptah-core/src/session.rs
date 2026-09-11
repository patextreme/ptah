//! The session façade: the types the script layer uses to talk to one
//! live agent session, independent of any transport.
//!
//! [`SessionHandle`] is already a good async façade (D4): it shuttles
//! commands over a channel and resolves outcomes through oneshots, so it
//! is the type both the ACP adapter produces and the script layer
//! consumes. The [`AgentTransport`](crate::ports::AgentTransport)
//! port is what creates handles.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    McpServer, SessionConfigOption, SessionConfigOptionValue, Usage,
};
use tokio::sync::{mpsc, oneshot};

use crate::contract::ResultContract;

/// Token counts reported for a turn.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageCounts {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl UsageCounts {
    pub fn from_usage(u: &Usage) -> Self {
        Self {
            input: u.input_tokens,
            cache_read: u.cached_read_tokens.unwrap_or(0),
            cache_write: u.cached_write_tokens.unwrap_or(0),
            output: u.output_tokens,
        }
    }
}

/// The result of one completed prompt turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOutcome {
    /// The turn's last agent message: the final contiguous run of streamed
    /// text, where tool-call activity ends a message run. Falls back to
    /// the previous non-empty run when the turn ends on tool activity
    /// with no trailing message; empty for cancelled and message-less
    /// turns. Intermediate messages still stream to the sink.
    pub text: String,
    /// `end_turn` | `max_tokens` | `max_turn_requests` | `refusal` | `cancelled`.
    pub stop_reason: String,
    /// Token counts (zero when unreported).
    pub usage: UsageCounts,
    /// The turn's last accepted typed submission, converted from JSON.
    /// `None` when the session declared no contract, the turn had no
    /// accepted submission, or the turn was cancelled/timed out.
    pub result: Option<serde_json::Value>,
}

/// Why a prompt turn failed.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnError {
    /// `timeoutMs` elapsed: `session/cancel` was sent, then the error raised.
    Timeout,
    /// The agent misbehaved or the protocol exchange failed.
    Agent(String),
    /// The connection closed (agent process died or was torn down).
    Closed(String),
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::Timeout => write!(f, "prompt timed out"),
            TurnError::Agent(e) => write!(f, "agent error: {e}"),
            TurnError::Closed(e) => write!(f, "connection closed: {e}"),
        }
    }
}

/// Errors starting or closing a session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// The configured agent command could not be spawned (names the command).
    Spawn(String),
    /// Handshake (`initialize` / `session/new`) failed.
    Handshake(String),
    /// The driver task died before the session became ready.
    DriverDied(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Spawn(msg) => write!(f, "failed to spawn agent command {msg}"),
            SessionError::Handshake(e) => write!(f, "agent handshake failed: {e}"),
            SessionError::DriverDied(e) => write!(f, "agent connection ended unexpectedly: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Options for creating one session.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// Working directory for the session (default: invocation directory).
    pub cwd: PathBuf,
    /// MCP servers passed through to the agent.
    pub mcp_servers: Vec<McpServer>,
    /// Attribution label, e.g. `claude/s1`.
    pub label: String,
    /// Typed result contract. When set, ptah injects the result-bridge
    /// MCP server into the session. Prompt text is passed through
    /// verbatim; submit guidance lives in the `result_submit` tool
    /// description.
    pub result: Option<ResultContract>,
}

/// Commands sent from Lua-side handles to the session driver.
pub enum SessionCmd {
    Prompt {
        text: String,
        timeout: Option<Duration>,
        resp: oneshot::Sender<Result<TurnOutcome, TurnError>>,
    },
    SetConfig {
        id: String,
        value: SessionConfigOptionValue,
        resp: oneshot::Sender<Result<(), String>>,
    },
    Cancel,
    Close,
}

/// A handle from the scripting side to one live agent session.
#[derive(Clone)]
pub struct SessionHandle {
    /// Attribution label (`agent/session`).
    pub label: String,
    /// The agent-assigned ACP session id (the id the agent generated
    /// in its `session/new` response). Opaque to ptah; meaningful to
    /// the agent's own tooling (resume commands, agent-side session
    /// listings). Stable for the session's lifetime.
    pub session_id: String,
    /// OS process id of the agent subprocess (for teardown assertions).
    pub pid: u32,
    pub cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    pub done_rx: tokio::sync::watch::Receiver<bool>,
    /// Serializes prompt turns on this session (cancellation does not take
    /// the lock).
    pub turn_lock: Arc<tokio::sync::Mutex<()>>,
    /// Live per-session config-option state (captured at `session/new`,
    /// then folded from `config_option_update` notifications and
    /// `set_config_option` responses).
    pub config_options: Arc<Mutex<Vec<SessionConfigOption>>>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("label", &self.label)
            .field("session_id", &self.session_id)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl SessionHandle {
    /// Send one prompt turn. Turns on the same session are serialized;
    /// `session:cancel()` works while a turn is in flight.
    pub async fn prompt(
        &self,
        text: String,
        timeout: Option<Duration>,
    ) -> Result<TurnOutcome, TurnError> {
        let _turn = self.turn_lock.lock().await;
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(SessionCmd::Prompt {
                text,
                timeout,
                resp: tx,
            })
            .map_err(|_| TurnError::Closed("session driver exited".into()))?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(TurnError::Closed("session driver exited".into())),
        }
    }

    /// Send `session/cancel` for the in-flight turn (if any).
    pub fn cancel(&self) {
        let _ = self.cmd_tx.send(SessionCmd::Cancel);
    }

    /// Snapshot the session's live config-option state.
    pub fn config_options(&self) -> Vec<SessionConfigOption> {
        self.config_options.lock().unwrap().clone()
    }

    /// Send one `session/set_config_option`. Serialized with prompt turns
    /// on this session via the turn lock, so config changes apply strictly
    /// between turns. Fails with a string carrying the config id and the
    /// agent's error message.
    pub async fn set_config(
        &self,
        id: String,
        value: SessionConfigOptionValue,
    ) -> Result<(), String> {
        let _turn = self.turn_lock.lock().await;
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(SessionCmd::SetConfig {
                id,
                value,
                resp: tx,
            })
            .map_err(|_| "session driver exited".to_string())?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err("session driver exited".to_string()),
        }
    }

    /// Ask the driver to close the session; does not wait.
    pub fn close(&self) {
        let _ = self.cmd_tx.send(SessionCmd::Close);
    }

    /// Wait for the driver (and the reaping of the agent process) to finish.
    pub async fn join(&self) {
        let mut rx = self.done_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                return; // sender dropped: treat as done
            }
        }
    }
}
