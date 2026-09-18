//! Terminal rendering of streaming agent output with per-session attribution.
//!
//! No TUI: plain stdout writes with a `[agent/session]` text prefix and a
//! per-session ANSI color assigned round-robin from a small palette.
//! `--no-color` drops the color codes; `--quiet` suppresses everything but
//! script `print` output; `-vv` additionally passes agent stderr through.
//!
//! [`record`] is the adapter beside the renderer: the durable run record
//! (`.ptah/runs/<id>/log` and `run.json`) and the fan-out sink that feeds
//! both.

pub mod record;

use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use std::time::Duration;

use ptah_core::events::{AskAction, PlanEntry, PlanStatus, SessionEvent};
use ptah_core::ports::EventSink;
use ptah_core::text::{LINE_BUDGET, truncate_visible};

/// Palette of distinct ANSI foreground hues, cycled per session label.
const PALETTE: [&str; 6] = [
    "\x1b[36m", // cyan
    "\x1b[33m", // yellow
    "\x1b[35m", // magenta
    "\x1b[32m", // green
    "\x1b[34m", // blue
    "\x1b[91m", // bright red
];

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

/// Local wall-clock time as `yyyy-mm-dd HH:MM:SS`, prefixed to every
/// rendered line. Taken at write time: the producers of display events
/// have no event time distinct from render time.
fn timestamp() -> String {
    let now = jiff::Zoned::now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// `--quiet`: suppress streaming render and diagnostics.
    pub quiet: bool,
    /// `--no-color`: emit text prefixes without ANSI sequences.
    pub no_color: bool,
    /// `-vv`: pass agent subprocess stderr through.
    pub agent_stderr: bool,
    /// `--verbose`: runtime lifecycle diagnostics.
    pub verbose: bool,
}

impl RenderOptions {
    /// All output suppressed (useful for tests).
    pub fn quiet() -> Self {
        Self {
            quiet: true,
            ..Self::default()
        }
    }
}

/// A display event extracted from a `session/update` notification.
#[derive(Debug)]
pub enum DisplayEvent {
    /// A chunk of the agent's message text (streamed).
    Chunk(String),
    /// One rendered tool line: the fully formatted body (`tool: <title>`
    /// at the call's start, `tool: <title> (<status>, <duration>)` when it
    /// settles). Transition policy and duration are decided where the
    /// update stream is folded; the renderer is a dumb sink.
    Tool(String),
    /// Compact plan status list.
    Plan(String),
    /// Context-window usage line.
    Usage { used: u64, size: u64 },
}

#[derive(Default)]
struct SessionBuf {
    /// Partial line buffered for the next newline (message chunks).
    partial: String,
}

struct Inner {
    out: BufWriter<Box<dyn Write + Send>>,
    styles: HashMap<String, String>,
    next_style: usize,
    bufs: HashMap<String, SessionBuf>,
}

/// Shared, thread-safe renderer.
pub struct Renderer {
    opts: RenderOptions,
    inner: Mutex<Inner>,
}

impl Renderer {
    pub fn new(opts: RenderOptions) -> Self {
        Self::with_writer(opts, std::io::stdout())
    }

    /// Build a renderer writing to an arbitrary sink instead of stdout
    /// (tests, embedders). Same buffering/flush semantics: one flush per
    /// completed line.
    pub fn with_writer<W: Write + Send + 'static>(opts: RenderOptions, out: W) -> Self {
        Self {
            opts,
            inner: Mutex::new(Inner {
                out: BufWriter::new(Box::new(out)),
                styles: HashMap::new(),
                next_style: 0,
                bufs: HashMap::new(),
            }),
        }
    }

    pub fn options(&self) -> RenderOptions {
        self.opts
    }

    fn style_for(&self, inner: &mut Inner, label: &str) -> String {
        if self.opts.no_color {
            return String::new();
        }
        inner
            .styles
            .entry(label.to_string())
            .or_insert_with(|| {
                let code = PALETTE[inner.next_style % PALETTE.len()].to_string();
                inner.next_style += 1;
                code
            })
            .clone()
    }

    fn prefixed_line(&self, inner: &mut Inner, label: &str, line: &str) {
        let ts = timestamp();
        if self.opts.no_color {
            let _ = writeln!(inner.out, "{ts} [{label}] {line}");
        } else {
            let style = self.style_for(inner, label);
            let _ = writeln!(inner.out, "{DIM}{ts}{RESET} [{label}]{style} {line}{RESET}");
        }
    }

    /// `[ptah]` diagnostic line (lifecycle, script log): timestamped but
    /// never label-colored.
    fn ptah_line(&self, inner: &mut Inner, msg: &str) {
        let ts = timestamp();
        if self.opts.no_color {
            let _ = writeln!(inner.out, "{ts} [ptah] {msg}");
        } else {
            let _ = writeln!(inner.out, "{DIM}{ts}{RESET} [ptah] {msg}");
        }
    }

    /// Write one complete prefixed line.
    pub fn line(&self, label: &str, text: &str) {
        if self.opts.quiet {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        for line in text.lines() {
            self.prefixed_line(&mut inner, label, line);
        }
        let _ = inner.out.flush();
    }

    /// Stream a message chunk; buffers partial lines so prefixes land on
    /// real lines. `flush` finishes any pending partial line (turn end).
    pub fn chunk(&self, label: &str, delta: &str, flush: bool) {
        if self.opts.quiet {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        let mut ready: Vec<String> = Vec::new();
        {
            let buf = inner.bufs.entry(label.to_string()).or_default();
            buf.partial.push_str(delta);
            while let Some(nl) = buf.partial.find('\n') {
                let line: String = buf.partial.drain(..=nl).collect();
                ready.push(line.trim_end_matches('\n').to_string());
            }
            if flush && !buf.partial.is_empty() {
                ready.push(std::mem::take(&mut buf.partial));
            }
        }
        for line in ready {
            self.prefixed_line(&mut inner, label, &line);
        }
        let _ = inner.out.flush();
    }

    /// Render a display event derived from a session update.
    pub fn event(&self, label: &str, event: DisplayEvent) {
        match event {
            DisplayEvent::Chunk(text) => self.chunk(label, &text, false),
            DisplayEvent::Tool(body) => self.line(label, &body),
            DisplayEvent::Plan(summary) => self.line(label, &summary),
            DisplayEvent::Usage { used, size } => {
                self.line(label, &format!("context: {used}/{size} tokens"));
            }
        }
    }

    /// `-vv`: pass one line of agent stderr through with attribution.
    pub fn agent_stderr(&self, label: &str, line: &str) {
        if !self.opts.agent_stderr || self.opts.quiet {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        self.prefixed_line(&mut inner, label, line);
        let _ = inner.out.flush();
    }

    /// `--verbose`: runtime lifecycle diagnostic.
    pub fn lifecycle(&self, msg: &str) {
        if self.opts.quiet || !self.opts.verbose {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        self.ptah_line(&mut inner, msg);
        let _ = inner.out.flush();
    }

    /// `ptah.exec` lifecycle line: script-level attribution (`[ptah]`),
    /// like [`Renderer::lifecycle`] but rendered in every non-quiet mode
    /// — exec lines are session-event-like visibility, not `--verbose`
    /// diagnostics, so a headless run never looks dead during a slow
    /// command.
    pub fn exec_line(&self, msg: &str) {
        if self.opts.quiet {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        self.ptah_line(&mut inner, msg);
        let _ = inner.out.flush();
    }

    /// `ptah.log`: script-initiated diagnostic on stdout (not suppressed by
    /// `--quiet`, which only silences streaming render/diagnostics).
    pub fn script_log(&self, msg: &str) {
        let mut inner = self.inner.lock().unwrap();
        self.ptah_line(&mut inner, msg);
        let _ = inner.out.flush();
    }

    /// Ask lines: required interaction — never suppressed by `--quiet`
    /// (a suppressed prompt is a hung run), timestamped like every
    /// rendered line, `--no-color` honored. Ask activity is attributed
    /// through the sink label (`ask {n} {script_basename}`) and reads as
    /// script activity, like exec lines — never as a named session.
    fn ask_line(&self, inner: &mut Inner, msg: &str) {
        self.ptah_line(inner, msg);
        let _ = inner.out.flush();
    }

    /// `AskRequested`: the ask's prose rows — the prompt's first line
    /// riding the label line, its further lines and any details lines
    /// indented beneath it — each through the timestamped `ask_line`
    /// path, then the `> ` input cue: written without a trailing
    /// newline and flushed, landing alone on the row after the last
    /// prose row (the user's own Enter terminates the visual line; over
    /// pipes the next rendered line simply follows). The provider first
    /// polls stdin only after this returns, so the prompt is on screen
    /// before input is read.
    fn ask_requested(&self, label: &str, prompt: &str, details: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        for line in ask_prose_lines(label, prompt, details) {
            self.ask_line(&mut inner, &line);
        }
        let _ = write!(inner.out, "> ");
        let _ = inner.out.flush();
    }

    /// `AskResolved`: one line carrying the ask number and the action —
    /// `respond` or `abort`, never the answer text (the terminal already
    /// shows what was typed).
    fn ask_resolved(&self, label: &str, action: AskAction) {
        let mut inner = self.inner.lock().unwrap();
        self.ask_line(&mut inner, &ask_resolved_line(label, action));
        let _ = inner.out.flush();
    }

    /// Flush buffered partial lines at turn end.
    pub fn flush_session(&self, label: &str) {
        if self.opts.quiet {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(buf) = inner.bufs.get_mut(label)
            && !buf.partial.is_empty()
        {
            let line = std::mem::take(&mut buf.partial);
            self.prefixed_line(&mut inner, label, &line);
        }
        let _ = inner.out.flush();
    }
}

/// One-line preview of a prompt for the prompt line: whitespace runs
/// collapsed to single spaces, truncated to the shared visible-char
/// budget.
fn prompt_preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_visible(&collapsed, LINE_BUDGET).into_owned()
}

/// Render marker for one plan status (matches the streaming plan line).
fn plan_marker(status: PlanStatus) -> char {
    match status {
        PlanStatus::Pending => ' ',
        PlanStatus::InProgress => '>',
        PlanStatus::Completed => 'x',
        PlanStatus::Other => '?',
    }
}

/// Compact plan status list for the plan line.
fn plan_summary(entries: &[PlanEntry]) -> String {
    let rendered: Vec<String> = entries
        .iter()
        .map(|e| format!("[{}] {}", plan_marker(e.status), e.content))
        .collect();
    format!("plan: {}", rendered.join(" "))
}

/// `X.Ys` under a minute, `Mm SS.Ss` above — the same shape tool lines
/// use, so durations read uniformly across the output.
fn format_duration(d: Duration) -> String {
    let tenths = (d.as_millis() + 50) / 100;
    if tenths < 600 {
        format!("{}.{}s", tenths / 10, tenths % 10)
    } else {
        format!(
            "{}m {:02}.{}s",
            tenths / 600,
            (tenths % 600) / 10,
            tenths % 10
        )
    }
}

/// A rendered ask row is blank when its source line is empty or
/// whitespace-only: blank lines carry no content to read, wherever
/// they sit.
fn ask_line_is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The authored lines of one ask prose field (prompt or details),
/// verbatim — no whitespace collapse, no truncation. Leading and
/// trailing blank lines are trimmed (display-side only; the event and
/// the run record keep the raw text); interior lines are kept exactly
/// as written, blank ones included. A text that is blank after
/// trimming yields no lines.
fn ask_prose_rows(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(first) = lines.iter().position(|l| !ask_line_is_blank(l)) else {
        return Vec::new();
    };
    // `first` exists, so `rposition` cannot miss; blank runs at both
    // edges fall away.
    let last = lines.iter().rposition(|l| !ask_line_is_blank(l)).unwrap();
    lines[first..=last].to_vec()
}

/// One indented ask continuation row: prompt lines after the first
/// and every details line render two spaces in under the label line.
/// A blank line renders empty — no indent padding, so no row is
/// padding-only.
fn ask_continuation(line: &str) -> String {
    if ask_line_is_blank(line) {
        String::new()
    } else {
        format!("  {line}")
    }
}

/// Every rendered row of one ask, verbatim. The prompt's first line
/// rides the label line (`{label}: {first}`); a prompt that is blank
/// after trimming leaves no text after the colon. Each further prompt
/// line and each details line renders as an indented continuation row
/// beneath it. Ask prose is exempt from the shared visible-char
/// budget: asks are required interaction, and an unreadable prompt is
/// a hung run in exactly the way a suppressed one is (the same
/// principle as the `--quiet` bypass).
fn ask_prose_lines(label: &str, prompt: &str, details: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    let rows = ask_prose_rows(prompt);
    if let Some((first, rest)) = rows.split_first() {
        lines.push(format!("{label}: {first}"));
        for line in rest {
            lines.push(ask_continuation(line));
        }
    } else {
        lines.push(format!("{label}:"));
    }
    if let Some(details) = details {
        for line in ask_prose_rows(details) {
            lines.push(ask_continuation(line));
        }
    }
    lines
}

/// Resolution line body: the attribution label plus the action
/// (`respond` / `abort`) — never the answer text.
fn ask_resolved_line(label: &str, action: AskAction) -> String {
    format!("{label}: {}", action.as_str())
}

/// One-line form of a command: whitespace runs collapsed to single
/// spaces (a multi-line command still renders as one line), truncated
/// to the shared visible-char budget.
fn exec_command_preview(command: &str) -> String {
    let collapsed = command.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_visible(&collapsed, LINE_BUDGET).into_owned()
}

/// Start line body for one exec: the command, collapsed and truncated to
/// the shared visible-char budget like prompt text.
fn exec_start_line(command: &str) -> String {
    format!("exec: {}", exec_command_preview(command))
}

/// End line body for one exec: command plus exit code + duration, or
/// the timeout/spawn-failure marker when no exit status exists.
fn exec_end_line(command: &str, exit_code: Option<i32>, timed_out: bool, d: Duration) -> String {
    let marker = match (exit_code, timed_out) {
        (Some(code), _) => format!("exit {code}"),
        (None, true) => "timed out".to_string(),
        (None, false) => "failed to run".to_string(),
    };
    format!(
        "exec: {} ({}, {})",
        exec_command_preview(command),
        marker,
        format_duration(d)
    )
}

/// The terminal renderer as an [`EventSink`]: structured session events
/// map onto the existing display-event handling; every byte of formatting
/// (truncation, prefixes, colors, gating) stays here.
impl EventSink for Renderer {
    fn emit(&self, label: &str, event: SessionEvent) {
        match event {
            SessionEvent::Prompt { text } => {
                self.line(label, &format!("prompt: {}", prompt_preview(&text)));
            }
            SessionEvent::TextDelta { delta, .. } => self.event(label, DisplayEvent::Chunk(delta)),
            SessionEvent::ToolLine(line) => self.event(label, DisplayEvent::Tool(line.body)),
            SessionEvent::Plan { entries } => {
                self.event(label, DisplayEvent::Plan(plan_summary(&entries)));
            }
            SessionEvent::Usage { used, size } => {
                self.event(label, DisplayEvent::Usage { used, size });
            }
            SessionEvent::StderrLine { line } => self.agent_stderr(label, &line),
            SessionEvent::Lifecycle { message } => self.lifecycle(&message),
            // Structured readiness: the renderer reconstructs the
            // verbose-only ready line from the event's facts (never from
            // the driver's wording). `agent`, `command`, `args`, and
            // `env_keys` are the record sink's business here.
            SessionEvent::SessionReady { label, acp_id, .. } => {
                self.lifecycle(&format!("{label}: session ready (acp {acp_id})"));
            }
            // Exec lifecycle: script-level `[ptah]` attribution (the
            // reserved "exec" pseudo-label marks these events at the
            // sink boundary; the label itself is not rendered).
            SessionEvent::ExecStart { command } => {
                self.exec_line(&exec_start_line(&command));
            }
            SessionEvent::ExecEnd {
                command,
                exit_code,
                timed_out,
                duration_ms,
            } => self.exec_line(&exec_end_line(
                &command,
                exit_code,
                timed_out,
                Duration::from_millis(duration_ms),
            )),
            // A structurally valid submission with no turn in flight is
            // dropped (not errored); the one-line note is the only render.
            SessionEvent::ResultVerdict { late: true, .. } => self.lifecycle(&format!(
                "{label}: dropped late typed-result submission (no turn in flight)"
            )),
            // Verdicts ride the result channel to the bridge; nothing to
            // render on the terminal sink.
            SessionEvent::ResultVerdict { .. } => {}
            // Ask lifecycle (required interaction — bypasses --quiet):
            // the label (`ask {n} {script}`) is the attribution.
            SessionEvent::AskRequested { prompt, details } => {
                self.ask_requested(label, &prompt, details.as_deref());
            }
            SessionEvent::AskResolved { action, .. } => self.ask_resolved(label, action),
            SessionEvent::TurnEnd => self.flush_session(label),
        }
    }

    fn script_log(&self, message: &str) {
        self.script_log(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `yyyy-mm-dd HH:MM:SS`: fixed width 19, digits and separators in
    /// the right slots (render-logging "Timestamp shape").
    #[test]
    fn timestamp_shape_is_date_prefixed() {
        let ts = timestamp();
        let b = ts.as_bytes();
        assert_eq!(ts.len(), 19, "fixed width: {ts:?}");
        let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
        assert!(digits(0..4) && b[4] == b'-', "year: {ts:?}");
        assert!(digits(5..7) && b[7] == b'-', "month: {ts:?}");
        assert!(digits(8..10) && b[10] == b' ', "day: {ts:?}");
        assert!(digits(11..13) && b[13] == b':', "hour: {ts:?}");
        assert!(digits(14..16) && b[16] == b':', "minute: {ts:?}");
        assert!(digits(17..19), "second: {ts:?}");
    }

    #[test]
    fn prompt_preview_collapses_and_truncates() {
        assert_eq!(
            prompt_preview("review\n  the\tauth\nmodule\n"),
            "review the auth module"
        );
        let long = "y".repeat(LINE_BUDGET + 10);
        assert_eq!(
            prompt_preview(&long),
            format!("{}\u{2026}", "y".repeat(LINE_BUDGET))
        );
    }

    #[test]
    fn plan_summary_renders_status_markers() {
        let entries = vec![
            PlanEntry {
                status: PlanStatus::Completed,
                content: "read the code".into(),
            },
            PlanEntry {
                status: PlanStatus::InProgress,
                content: "fix the bug".into(),
            },
        ];
        assert_eq!(
            plan_summary(&entries),
            "plan: [x] read the code [>] fix the bug"
        );
    }

    // Exec lifecycle lines (render-logging "Exec lines render command
    // and outcome"): start carries the command, end carries exit code +
    // duration or the timeout/spawn-failure marker.
    #[test]
    fn exec_start_line_carries_the_command() {
        assert_eq!(exec_start_line("printf hi"), "exec: printf hi");
        // Whitespace runs collapse: a multi-line command renders as one
        // line (one event, one rendered line).
        assert_eq!(
            exec_start_line("printf 'a\nb\nc'\n | wc -l"),
            "exec: printf 'a b c' | wc -l"
        );
        let long = "x".repeat(LINE_BUDGET + 5);
        assert_eq!(
            exec_start_line(&long),
            format!("exec: {}…", "x".repeat(LINE_BUDGET))
        );
    }

    #[test]
    fn exec_end_line_carries_code_or_marker() {
        assert_eq!(
            exec_end_line("true", Some(0), false, Duration::from_millis(980)),
            "exec: true (exit 0, 1.0s)"
        );
        assert_eq!(
            exec_end_line("sh -c 'exit 4'", Some(4), false, Duration::from_millis(50)),
            "exec: sh -c 'exit 4' (exit 4, 0.1s)"
        );
        assert_eq!(
            exec_end_line("sleep 5", None, true, Duration::from_millis(100)),
            "exec: sleep 5 (timed out, 0.1s)"
        );
        assert_eq!(
            exec_end_line(
                "definitely-not-a-command",
                None,
                false,
                Duration::from_millis(3)
            ),
            "exec: definitely-not-a-command (failed to run, 0.0s)"
        );
    }

    #[test]
    fn exec_duration_formats_like_tool_lines() {
        assert_eq!(format_duration(Duration::from_millis(0)), "0.0s");
        assert_eq!(format_duration(Duration::from_millis(2900)), "2.9s");
        // 59.96s rounds up-front so seconds never display 60.0.
        assert_eq!(format_duration(Duration::from_millis(59_960)), "1m 00.0s");
        assert_eq!(format_duration(Duration::from_millis(65_000)), "1m 05.0s");
    }

    // ------------------------------------------------------------------
    // Ask lines (ask capability / render-logging "Ask lines render
    // under the ask label")
    // ------------------------------------------------------------------

    /// A shared buffer the renderer can write to, readable from the
    /// test while the renderer is alive.
    #[derive(Clone, Default)]
    struct SharedOut(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl SharedOut {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl Write for SharedOut {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn ask_line_bodies_carry_attribution_and_action() {
        // Prompt: first line rides the label line, interior whitespace
        // stays verbatim (no collapse); resolution: label plus the
        // action, never the text.
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "Blocked: how   to\ncontinue?", None),
            ["ask 1 main.luau: Blocked: how   to", "  continue?",]
        );
        assert_eq!(
            ask_resolved_line("ask 2 main.luau", AskAction::Respond),
            "ask 2 main.luau: respond"
        );
        assert_eq!(
            ask_resolved_line("ask 3 main.luau", AskAction::Abort),
            "ask 3 main.luau: abort"
        );
    }

    #[test]
    fn ask_prose_renders_in_full_beyond_the_shared_budget() {
        // Ask prose is exempt from the shared visible-char budget: a
        // prompt or details line longer than LINE_BUDGET renders in
        // full — no `…` truncation marker on any ask line.
        let label = "ask 1 main.luau";
        let long = "y".repeat(LINE_BUDGET + 10);
        assert_eq!(
            ask_prose_lines(label, &long, None),
            [format!("{label}: {long}")]
        );
        assert_eq!(
            ask_prose_lines(label, "Proceed?", Some(&long)),
            [
                String::from("ask 1 main.luau: Proceed?"),
                format!("  {long}")
            ]
        );
    }

    #[test]
    fn ask_multi_line_prompt_indents_continuations() {
        // First line on the label, each authored further line one
        // indented row beneath it.
        assert_eq!(
            ask_prose_lines(
                "ask 1 main.luau",
                "How to continue?\nPick one:\n1. retry\n2. abort",
                None
            ),
            [
                "ask 1 main.luau: How to continue?",
                "  Pick one:",
                "  1. retry",
                "  2. abort",
            ]
        );
    }

    #[test]
    fn ask_details_render_as_an_indented_block() {
        // Each details line is one indented row beneath the prompt,
        // multi-line details included.
        assert_eq!(
            ask_prose_lines(
                "ask 1 main.luau",
                "Proceed?",
                Some("probe output …\nexit 3")
            ),
            ["ask 1 main.luau: Proceed?", "  probe output …", "  exit 3",]
        );
    }

    #[test]
    fn ask_interior_blank_lines_render_empty() {
        // Authored structure preserved: an interior blank line renders
        // as an empty row — no indent padding. Whitespace-only lines
        // are blank.
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "Question?\n\nOptions follow", None),
            ["ask 1 main.luau: Question?", "", "  Options follow"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "Question?\n   \nOptions", None),
            ["ask 1 main.luau: Question?", "", "  Options"]
        );
    }

    #[test]
    fn ask_leading_and_trailing_blank_lines_trim() {
        // `"Question?\n"` gains no dangling empty row before the cue;
        // a leading blank line puts no empty payload on the label row;
        // blank runs at both edges fall away entirely. Details trim the
        // same way.
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "Question?\n", None),
            ["ask 1 main.luau: Question?"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "\nQuestion?", None),
            ["ask 1 main.luau: Question?"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "\n\nA\n\nB\n\n\n", None),
            ["ask 1 main.luau: A", "", "  B"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "Proceed?", Some("hint\n")),
            ["ask 1 main.luau: Proceed?", "  hint"]
        );
    }

    #[test]
    fn ask_blank_prompt_renders_the_bare_label_line() {
        // An empty or all-blank prompt leaves no text after the colon;
        // details still render beneath the bare label row, and an
        // all-blank details block renders no rows.
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "", None),
            ["ask 1 main.luau:"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", " \n\t\n", None),
            ["ask 1 main.luau:"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "", Some("decide now")),
            ["ask 1 main.luau:", "  decide now"]
        );
        assert_eq!(
            ask_prose_lines("ask 1 main.luau", "Proceed?", Some(" \n ")),
            ["ask 1 main.luau: Proceed?"]
        );
    }

    #[test]
    fn ask_events_render_verbatim_prose_and_resolution() {
        // Full event path through a real renderer over a shared
        // buffer: label line, indented continuations (interior blank
        // line empty), indented details block — each row timestamped
        // and `[ptah]`-attributed — then the resolution line with the
        // action only; the answer text never appears in any
        // ptah-rendered line.
        let out = SharedOut::default();
        let renderer = Renderer::with_writer(RenderOptions::default(), out.clone());
        renderer.emit(
            "ask 1 main.luau",
            SessionEvent::AskRequested {
                prompt: "Blocked: how to continue?\nPick one:\n\n1. retry\n2. abort".into(),
                details: Some("probe output …".into()),
            },
        );
        renderer.emit(
            "ask 1 main.luau",
            SessionEvent::AskResolved {
                action: AskAction::Respond,
                text: Some("secret answer".into()),
            },
        );
        let text = out.text();
        let stripped = crate_test_strip(&text);
        // The blank interior row carries attribution only (no indent
        // padding), exactly like every other row.
        let expected = [
            "[ptah] ask 1 main.luau: Blocked: how to continue?",
            "[ptah]   Pick one:",
            "[ptah] ",
            "[ptah]   1. retry",
            "[ptah]   2. abort",
            "[ptah]   probe output …",
        ]
        .join("\n");
        assert!(stripped.contains(&expected), "{text}");
        assert!(
            stripped.contains("[ptah] ask 1 main.luau: respond"),
            "{text}"
        );
        assert!(
            !stripped.contains("secret answer"),
            "answer must not re-echo: {text}"
        );
    }

    #[test]
    fn ask_cue_lands_on_the_row_after_the_last_prose_line() {
        // Every prose row is newline-terminated; the `> ` cue is the
        // final write, with no newline of its own — alone on the next
        // row, so the user's typed answer starts at the cue.
        let out = SharedOut::default();
        let renderer = Renderer::with_writer(RenderOptions::default(), out.clone());
        renderer.ask_requested(
            "ask 1 main.luau",
            "Multi-line\nquestion\n\nfollow-up",
            Some("detail line"),
        );
        let text = out.text();
        let cue = text.find("> ").expect("cue present");
        assert_eq!(&text[cue..], "> ", "cue is the final write: {text:?}");
        assert!(
            text[..cue].ends_with('\n'),
            "cue follows the last prose row's newline: {text:?}"
        );
    }

    #[test]
    fn ask_lines_bypass_quiet() {
        // `--quiet` suppresses streaming and diagnostics — asks are
        // required interaction, gated like `script_log`, never silent.
        let out = SharedOut::default();
        let renderer = Renderer::with_writer(RenderOptions::quiet(), out.clone());
        renderer.ask_requested("ask 1 main.luau", "Proceed?", None);
        renderer.ask_resolved("ask 1 main.luau", AskAction::Abort);
        let text = out.text();
        assert!(text.contains("ask 1 main.luau: Proceed?"), "{text}");
        assert!(text.contains("ask 1 main.luau: abort"), "{text}");
    }

    #[test]
    fn ask_lines_follow_no_color() {
        let out = SharedOut::default();
        let renderer = Renderer::with_writer(
            RenderOptions {
                no_color: true,
                ..RenderOptions::default()
            },
            out.clone(),
        );
        renderer.ask_requested("ask 1 main.luau", "Proceed?", None);
        let text = out.text();
        assert!(!text.contains('\u{1b}'), "no ANSI escapes: {text:?}");
        assert!(text.contains("[ptah] ask 1 main.luau: Proceed?"), "{text}");
    }

    #[test]
    fn session_ready_renders_verbose_only() {
        // render-logging "Session-ready line names the ACP session id":
        // the renderer reconstructs the line from the structured event
        // (verbose-only, `{label}: session ready (acp {id})`).
        let event = || SessionEvent::SessionReady {
            label: "mock/s1".into(),
            agent: "mock".into(),
            command: "mock-agent".into(),
            args: vec!["--flag".into()],
            env_keys: vec!["TOKEN".into()],
            acp_id: "acp-123".into(),
        };

        let out = SharedOut::default();
        let renderer = Renderer::with_writer(
            RenderOptions {
                verbose: true,
                ..RenderOptions::default()
            },
            out.clone(),
        );
        renderer.emit("mock/s1", event());
        let stripped = crate_test_strip(&out.text());
        assert!(
            stripped.contains("[ptah] mock/s1: session ready (acp acp-123)"),
            "{stripped}"
        );

        let out = SharedOut::default();
        let renderer = Renderer::with_writer(RenderOptions::default(), out.clone());
        renderer.emit("mock/s1", event());
        assert!(
            !out.text().contains("session ready"),
            "default mode must suppress the line: {}",
            out.text()
        );
    }

    /// Strip the leading timestamp — `yyyy-mm-dd HH:MM:SS ` plain, or
    /// wrapped in the dim/`RESET` pair in colored mode — from every
    /// line (test-local; the integration suite has its own).
    fn crate_test_strip(output: &str) -> String {
        output
            .lines()
            .map(|l| {
                // `{DIM}{ts}{RESET} `: 4 + 19 + 4 bytes, then the space.
                if l.starts_with(DIM) && l.len() >= 28 && l.as_bytes()[27] == b' ' {
                    &l[28..]
                } else if l.len() >= 20 && l.as_bytes()[19] == b' ' {
                    &l[20..]
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
