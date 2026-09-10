//! Structured domain events: what one live agent session did, in the
//! order it happened.
//!
//! The session driver folds wire updates (via [`crate::turn`]) and
//! emits these through the [`EventSink`](crate::ports::EventSink)
//! port; renderers and future sinks format them. Payloads carry the
//! structured facts (ids, kinds, statuses, counts) so a TUI can track
//! state without parsing display strings — all formatting (truncation,
//! budgets, prefixes, colors) belongs to the sink implementation.

use agent_client_protocol::schema::v1::ToolKind;

/// One event from a live agent session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// A prompt turn was sent; `text` is the raw prompt.
    Prompt { text: String },
    /// A chunk of the agent's streamed message text. `message_break`
    /// marks that tool-call activity ended the previous message run
    /// before this chunk (message-boundary metadata; the line renderer
    /// ignores it).
    TextDelta { delta: String, message_break: bool },
    /// One rendered tool line, folded by the session's tool policy
    /// (transition dedup + duration).
    ToolLine(ToolLine),
    /// A plan update: entries in order.
    Plan { entries: Vec<PlanEntry> },
    /// Context-window usage report.
    Usage { used: u64, size: u64 },
    /// One line of agent subprocess stderr.
    StderrLine { line: String },
    /// Runtime lifecycle diagnostic (session readiness, config changes,
    /// typed-result setup, teardown notes).
    Lifecycle { message: String },
    /// A `ptah.exec` command started. Attributed to the script (the
    /// sink's reserved `"exec"` pseudo-label), not to any session.
    ExecStart { command: String },
    /// A `ptah.exec` command ended: its exit code (or, with `None`, a
    /// timeout/spawn-failure marker — `timed_out` discriminates) and
    /// wall-clock duration.
    ExecEnd {
        command: String,
        exit_code: Option<i32>,
        timed_out: bool,
        duration_ms: u64,
    },
    /// The verdict for one typed-result submission. `late` marks a
    /// structurally valid submission that arrived with no turn in flight
    /// (dropped, not an error).
    ResultVerdict { accepted: bool, late: bool },
    /// A `ptah.ask` was issued to the human: the prompt and optional
    /// details, attributed through the sink label (`ask {n} {script}`).
    /// Ask events are required interaction: sinks render them even in
    /// quiet modes (a suppressed prompt is a hung run).
    AskRequested {
        prompt: String,
        details: Option<String>,
    },
    /// A `ptah.ask` resolved: the action taken and, for `respond`, the
    /// full answer text (for downstream sinks; the terminal renderer
    /// never re-echoes it). Emitted only for resolutions — an ask
    /// dropped by run teardown emits no resolution.
    AskResolved {
        action: AskAction,
        text: Option<String>,
    },
    /// The turn's stream ended: flush any partial line buffers.
    TurnEnd,
}

/// How one ask resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskAction {
    /// The human answered (the text rides [`SessionEvent::AskResolved`]).
    Respond,
    /// The human chose the provider's abort gesture.
    Abort,
}

impl AskAction {
    /// The wire name (`"respond"` / `"abort"`) — also the result table's
    /// `action` value in scripts.
    pub fn as_str(self) -> &'static str {
        match self {
            AskAction::Respond => "respond",
            AskAction::Abort => "abort",
        }
    }
}

/// One folded tool-call line: the fully formatted body plus the
/// structured facts it was built from.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolLine {
    /// The call's id (update correlation key).
    pub id: String,
    /// The effective title: the announced title, or the raw call id for
    /// updates that preceded their announcement.
    pub title: String,
    /// The call's folded kind, when one was carried.
    pub kind: Option<ToolKind>,
    /// The status whose transition rendered this line.
    pub status: String,
    /// The formatted line body (`tool: <title> [<peek>] [(<status>,
    /// <duration>)]`).
    pub body: String,
}

/// One plan entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanEntry {
    pub status: PlanStatus,
    pub content: String,
}

/// Plan entry status (protocol-agnostic; sinks render their own marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
    Other,
}
