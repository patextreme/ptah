//! Terminal rendering of streaming agent output with per-session attribution.
//!
//! No TUI: plain stdout writes with a `[agent/session]` text prefix and a
//! per-session ANSI color assigned round-robin from a small palette.
//! `--no-color` drops the color codes; `--quiet` suppresses everything but
//! script `print` output; `-vv` additionally passes agent stderr through.

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

    /// `AskRequested`: the prompt line, an indented details line when
    /// present, then the `> ` input cue — written without a trailing
    /// newline and flushed (the user's own Enter terminates the visual
    /// line; over pipes the next rendered line simply follows). The
    /// provider first polls stdin only after this returns, so the
    /// prompt is on screen before input is read.
    fn ask_requested(&self, label: &str, prompt: &str, details: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        self.ask_line(&mut inner, &ask_prompt_line(label, prompt));
        if let Some(details) = details {
            self.ask_line(&mut inner, &ask_details_line(details));
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

/// Prompt line body for one ask: the attribution label plus the
/// prompt, whitespace-collapsed and truncated under the shared
/// visible-char budget (the same mechanics as prompt lines).
fn ask_prompt_line(label: &str, prompt: &str) -> String {
    format!("{label}: {}", prompt_preview(prompt))
}

/// Details line body for one ask: an indented continuation line under
/// the prompt, same collapse/truncate mechanics.
fn ask_details_line(details: &str) -> String {
    format!("  {}", prompt_preview(details))
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
        // Prompt: label plus collapsed prompt; details: indented
        // continuation; resolution: label plus action, never the text.
        assert_eq!(
            ask_prompt_line("ask 1 main.luau", "Blocked: how   to\ncontinue?"),
            "ask 1 main.luau: Blocked: how to continue?"
        );
        assert_eq!(ask_details_line("probe output …"), "  probe output …");
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
    fn ask_prompt_and_details_truncate_under_the_shared_budget() {
        let label = "ask 1 main.luau";
        let long = "y".repeat(LINE_BUDGET + 10);
        assert_eq!(
            ask_prompt_line(label, &long),
            format!("{label}: {}…", "y".repeat(LINE_BUDGET))
        );
        assert_eq!(
            ask_details_line(&long),
            format!("  {}…", "y".repeat(LINE_BUDGET))
        );
    }

    #[test]
    fn ask_events_render_prompt_details_cue_and_resolution() {
        // Full event path through a real renderer over a shared buffer:
        // prompt line, indented details line, `> ` cue without newline,
        // resolution line with the action only — the answer text never
        // appears in any ptah-rendered line.
        let out = SharedOut::default();
        let renderer = Renderer::with_writer(RenderOptions::default(), out.clone());
        renderer.emit(
            "ask 1 main.luau",
            SessionEvent::AskRequested {
                prompt: "Blocked: how to continue?".into(),
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
        assert!(stripped.contains("[ptah] ask 1 main.luau: Blocked: how to continue?"), "{text}");
        assert!(stripped.contains("[ptah]   probe output …"), "{text}");
        // The cue: no newline after it, and flushed.
        assert!(text.ends_with("> ") || text.contains("> "), "{text}");
        assert!(stripped.contains("[ptah] ask 1 main.luau: respond"), "{text}");
        assert!(!stripped.contains("secret answer"), "answer must not re-echo: {text}");
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

    /// Strip the leading `yyyy-mm-dd HH:MM:SS ` timestamp from every
    /// line (test-local; the integration suite has its own).
    fn crate_test_strip(output: &str) -> String {
        output
            .lines()
            .map(|l| if l.len() >= 20 && l.as_bytes()[19] == b' ' { &l[20..] } else { l })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
