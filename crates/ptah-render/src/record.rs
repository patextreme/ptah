//! The run record: the durable, self-describing artifact of one `ptah run`.
//!
//! A record is an output adapter that sits beside the terminal renderer and
//! shares its [`RenderOptions`](crate::RenderOptions). The record's `log` is
//! the run's rendered stream at the run's verbosity, never silenced by
//! `--quiet` and never colored; `run.json` is rewritten atomically every time
//! ptah learns a fact it carries. This module owns the run id, the record's
//! location and its self-ignoring `.gitignore`, the `run.json` schema, and the
//! fan-out sink that feeds both renderers.
//!
//! This is the filesystem adapter side of the hexagon — the domain in
//! `ptah-core` stays I/O-free. `ptah-render` needs neither `gix` nor `sha2`:
//! the record pins invocation *shape*, never code provenance.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use ptah_core::events::{AskAction, SessionEvent};
use ptah_core::ports::EventSink;
use serde::{Deserialize, Serialize};

use crate::{RenderOptions, Renderer};

/// The `run.json` layout version. Bumped when the record's shape changes
/// incompatibly, so a reader can tell one layout from another.
pub const SCHEMA_VERSION: u32 = 1;

/// The project root used for a record: the nearest ancestor of
/// `invocation_dir` (including itself) containing a `.ptah` *directory*, or
/// `invocation_dir` when no ancestor has one.
///
/// This matches `package-management`'s definition of the project root (the
/// directory containing `.ptah/`) rather than `ptah-config`'s
/// nearest-`.ptah/config.toml` search, so the workspace keeps one notion of
/// project root.
pub fn project_root(invocation_dir: &Path) -> PathBuf {
    for ancestor in invocation_dir.ancestors() {
        if ancestor.join(".ptah").is_dir() {
            return ancestor.to_path_buf();
        }
    }
    invocation_dir.to_path_buf()
}

/// The record directory's parent under a project root: `<project>/.ptah/runs`.
///
/// Public so the composition root can name the location in the
/// record-unavailable warning.
pub fn runs_dir(invocation_dir: &Path) -> PathBuf {
    project_root(invocation_dir).join(".ptah").join("runs")
}

/// A path rendered for display under the render-logging path rule: relative
/// to `invocation_dir` when under it, collapsed to `~` when under `home` but
/// not under the invocation directory, and as received otherwise. The same
/// rule peek paths follow; pure so all three forms are unit-tested.
pub fn shorten_path(path: &Path, invocation_dir: &Path, home: Option<&Path>) -> String {
    if let Ok(rel) = path.strip_prefix(invocation_dir)
        && !rel.as_os_str().is_empty()
    {
        return rel.display().to_string();
    }
    if let Some(home) = home
        && let Ok(rel) = path.strip_prefix(home)
        && !rel.as_os_str().is_empty()
    {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

/// The run-start line body: a `ptah`-attributed line naming the record
/// directory under the path rule above (and therefore the run id). Emitted
/// through the `exec_line` gate, so it renders in every non-quiet terminal
/// mode and is always written to the record's `log` (the record renderer is
/// never quiet).
pub fn run_start_line(record_dir: &Path, invocation_dir: &Path, home: Option<&Path>) -> String {
    format!(
        "run record: {}",
        shorten_path(record_dir, invocation_dir, home)
    )
}

/// A run's identity: the UTC start instant as `yyyymmddhhmmss`, a `-`, and a
/// short decimal suffix that breaks same-second collisions. The timestamp
/// prefix sorts lexicographically in start order across seconds; two runs that
/// start within the same second share the prefix, and their suffix order is
/// deliberately unspecified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunId(String);

impl RunId {
    /// Mint an id for a run starting at `start`, re-minting while a directory
    /// of that id already exists under `runs_dir` (an occupied id is never
    /// reused or merged into). The prefix is formatted from the absolute
    /// instant in UTC, so it is independent of the machine's local zone.
    pub fn mint(start: Timestamp, runs_dir: &Path) -> io::Result<RunId> {
        let prefix = start.strftime("%Y%m%d%H%M%S").to_string();
        loop {
            let suffix = getrandom::u32().map_err(io::Error::other)? % 10_000;
            let id = format!("{prefix}-{suffix:04}");
            if !runs_dir.join(&id).exists() {
                return Ok(RunId(id));
            }
        }
    }

    /// The id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a run ended, flattened from the runtime's `RunOutcome` (ptah-render
/// must not depend on `ptah-luau`, so the composition root copies the fields
/// across the boundary). `cancelled` is the runtime's separate report that a
/// terminating signal ended the run — never inferred from `code`, so a
/// script's own `ptah.exit(130)` stays a failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunEnd {
    pub code: i32,
    pub error: Option<String>,
    pub undelivered_errors: Vec<String>,
    pub cancelled: bool,
}

impl RunEnd {
    /// `cancelled` when a signal terminated the run, else `ok` for exit 0 and
    /// `failed` for any non-zero exit.
    pub fn status(&self) -> RunStatus {
        if self.cancelled {
            RunStatus::Cancelled
        } else if self.code == 0 {
            RunStatus::Ok
        } else {
            RunStatus::Failed
        }
    }

    /// The terminal error message: the escaped/undelivered error, joined when
    /// a task error was never observed. `None` when the run ended cleanly.
    pub fn error_text(&self) -> Option<String> {
        match (&self.error, self.undelivered_errors.is_empty()) {
            (Some(e), _) => Some(e.clone()),
            (None, false) => Some(self.undelivered_errors.join("; ")),
            (None, true) => None,
        }
    }
}

/// How the run ended, as `run.json` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    /// The run has not ended (also the honest state of a run killed without
    /// teardown: SIGKILL, or a second SIGINT/SIGTERM).
    Running,
    Ok,
    Failed,
    Cancelled,
}

/// One session that started: its label, agent name, authored invocation
/// shape, and the agent-assigned ACP session id — populated from the
/// structured [`SessionEvent::SessionReady`] fact, never parsed from a
/// rendered line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub label: String,
    pub agent: String,
    /// The authored (pre-interpolation) command.
    pub command: String,
    /// The authored (pre-interpolation) args.
    pub args: Vec<String>,
    /// The declared environment key *names* only — never values.
    pub env_keys: Vec<String>,
    pub acp_id: String,
}

/// One ask issued during the run. `ordinal` is the record's own emission-order
/// count of issued asks; `action`/`text` are `None` until the ask resolves (an
/// ask dropped by teardown keeps its prompt and no resolution).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskRecord {
    pub ordinal: usize,
    pub prompt: String,
    pub details: Option<String>,
    pub action: Option<String>,
    pub text: Option<String>,
}

/// The `run.json` model: the run's identity and shape, and the sessions and
/// asks it accumulated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMeta {
    pub schema_version: u32,
    pub run_id: String,
    /// The entry script's path, as the process received it.
    pub script: String,
    /// The process argv.
    pub argv: Vec<String>,
    /// The invocation directory.
    pub invocation_dir: String,
    /// The ptah version string.
    pub ptah_version: String,
    /// Start instant, RFC 3339 in UTC.
    pub started_at: String,
    /// End instant, RFC 3339 in UTC; `None` while the run is live.
    pub ended_at: Option<String>,
    pub status: RunStatus,
    /// The process exit code, once the run has ended.
    pub exit_code: Option<i32>,
    /// The terminal error message when the run ended in error.
    pub error: Option<String>,
    pub sessions: Vec<SessionRecord>,
    pub asks: Vec<AskRecord>,
}

/// A live run record. Created before the script executes; each mutating method
/// rewrites `run.json` atomically as the run learns a fact. Recording is
/// best-effort: methods return the write error so the composition root can
/// warn once and carry on, never abort the run.
pub struct RunRecord {
    id: RunId,
    dir: PathBuf,
    state: Mutex<RecordState>,
    /// The first `log` write failure, stashed by the `LogWriter`. The `log`
    /// is written by a [`Renderer`], which discards write errors (a render
    /// never fails a run), so the writer records the first one here for the
    /// composition root's single warning.
    log_failure: Arc<Mutex<Option<String>>>,
}

struct RecordState {
    meta: RunMeta,
    next_ask: usize,
    /// The first write failure's reason, taken once by the composition
    /// root for its single warning.
    failure: Option<String>,
    /// Set by the first write failure: the record is disabled for the rest
    /// of the run, so no further write is attempted even after the reason
    /// has been taken.
    disabled: bool,
}

impl RunRecord {
    /// Create the record for a run that starts now.
    pub fn create(
        invocation_dir: &Path,
        script: &Path,
        argv: &[String],
    ) -> io::Result<RunRecord> {
        Self::create_at(invocation_dir, script, argv, Timestamp::now())
    }

    /// Like [`RunRecord::create`] with an explicit start instant (tests).
    pub fn create_at(
        invocation_dir: &Path,
        script: &Path,
        argv: &[String],
        start: Timestamp,
    ) -> io::Result<RunRecord> {
        let runs = runs_dir(invocation_dir);
        fs::create_dir_all(&runs)?;
        // The record directory and its ignore file are one unit. Write the
        // ignore file first, so a record directory can never appear without
        // it; an existing ignore file is left untouched. A record that could
        // leak into a tracked `.ptah/` commit is worse than no record.
        let ignore = runs.join(".gitignore");
        if !ignore.is_file() {
            fs::write(&ignore, "*\n")?;
        }
        let id = RunId::mint(start, &runs)?;
        let dir = runs.join(id.as_str());
        // Build the record in a sibling temp directory and publish it with a
        // single rename, so the run id never names a directory missing either
        // file: an abnormal kill during creation leaves only a temp
        // directory, never a half record under the id.
        let tmp = runs.join(format!(".{}.tmp", id.as_str()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir(&tmp)?;
        let meta = RunMeta {
            schema_version: SCHEMA_VERSION,
            run_id: id.as_str().to_string(),
            script: script.display().to_string(),
            argv: argv.to_vec(),
            invocation_dir: invocation_dir.display().to_string(),
            ptah_version: ptah_core::VERSION.to_string(),
            started_at: rfc3339(start),
            ended_at: None,
            status: RunStatus::Running,
            exit_code: None,
            error: None,
            sessions: Vec::new(),
            asks: Vec::new(),
        };
        // The two files must both exist before the id is visible. A failure
        // in any step leaves no record directory under the id (only, at
        // most, the temp directory).
        let formed = write_meta(&tmp, &meta)
            .and_then(|()| File::create(tmp.join("log")).map(|_| ()))
            .and_then(|()| fs::rename(&tmp, &dir));
        if let Err(e) = formed {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
        Ok(RunRecord {
            id,
            dir,
            state: Mutex::new(RecordState {
                meta,
                next_ask: 0,
                failure: None,
                disabled: false,
            }),
            log_failure: Arc::new(Mutex::new(None)),
        })
    }

    /// The run id.
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// The record directory (`<project>/.ptah/runs/<id>`).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A renderer writing the run's rendered stream into the record's `log`,
    /// built from the terminal's verbosity but never silenced and never
    /// colored (`quiet: false`, `no_color: true`). `--quiet` governs the
    /// terminal alone. Write failures are stashed by the `LogWriter` so
    /// [`RunRecord::take_failure`] can report them once.
    pub fn log_renderer(&self, terminal: RenderOptions) -> io::Result<Renderer> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("log"))?;
        let opts = RenderOptions {
            quiet: false,
            no_color: true,
            ..terminal
        };
        Ok(Renderer::with_writer(
            opts,
            LogWriter {
                inner: file,
                failure: self.log_failure.clone(),
            },
        ))
    }

    /// Record a session's readiness: label, agent name, authored invocation
    /// shape, and ACP session id.
    pub fn session_ready(&self, session: SessionRecord) -> io::Result<()> {
        self.update(|st| st.meta.sessions.push(session))
    }

    /// Record an ask as it is issued, assigning its ordinal from the record's
    /// own emission-order count (asks are serialized in issue order, so this
    /// matches the rendered `ask {n}`).
    pub fn ask_requested(&self, prompt: &str, details: Option<&str>) -> io::Result<()> {
        self.update(|st| {
            st.next_ask += 1;
            st.meta.asks.push(AskRecord {
                ordinal: st.next_ask,
                prompt: prompt.to_string(),
                details: details.map(str::to_string),
                action: None,
                text: None,
            });
        })
    }

    /// Record an ask's resolution: the action and, for `respond`, the full
    /// response text. Resolves the most recent still-open ask.
    pub fn ask_resolved(&self, action: AskAction, text: Option<&str>) -> io::Result<()> {
        self.update(|st| {
            if let Some(ask) = st.meta.asks.iter_mut().rev().find(|a| a.action.is_none()) {
                ask.action = Some(action.as_str().to_string());
                ask.text = text.map(str::to_string);
            }
        })
    }

    /// Close the record: end instant, status (cancellation takes priority over
    /// the exit code), exit code, and terminal error text.
    pub fn finish(&self, end: &RunEnd) -> io::Result<()> {
        self.update(|st| {
            st.meta.ended_at = Some(rfc3339(Timestamp::now()));
            st.meta.status = end.status();
            st.meta.exit_code = Some(end.code);
            st.meta.error = end.error_text();
        })
    }

    /// Serialize the current model and atomically replace `run.json`. A
    /// record already disabled by an earlier write failure stays disabled.
    pub fn rewrite(&self) -> io::Result<()> {
        let st = self.state.lock().unwrap();
        if st.disabled {
            return Ok(());
        }
        write_meta(&self.dir, &st.meta)
    }

    /// The first record-path write failure, taken once: a `run.json` write
    /// failure (which disables the record) or a `log` write failure stashed
    /// by the `LogWriter`. `None` when the record has written cleanly so
    /// far.
    pub fn take_failure(&self) -> Option<String> {
        if let Some(reason) = self.log_failure.lock().unwrap().take() {
            return Some(reason);
        }
        self.state.lock().unwrap().failure.take()
    }

    /// The current model (tests and inspection).
    pub fn meta(&self) -> RunMeta {
        self.state.lock().unwrap().meta.clone()
    }

    /// Mutate the model and persist it. The first failed write stashes the
    /// reason (for the composition root's single warning) and disables the
    /// record: no further write is attempted, so a record that has gone
    /// unwritable cannot repeatedly fail. Best-effort means never letting a
    /// record problem touch the run.
    fn update(&self, f: impl FnOnce(&mut RecordState)) -> io::Result<()> {
        let mut st = self.state.lock().unwrap();
        if st.disabled {
            // Disabled: the record stopped at its first write failure.
            return Ok(());
        }
        f(&mut st);
        match write_meta(&self.dir, &st.meta) {
            Ok(()) => Ok(()),
            Err(e) => {
                st.disabled = true;
                st.failure = Some(e.to_string());
                Err(e)
            }
        }
    }
}

/// Wraps the record's `log` file. A [`Renderer`] discards write and flush
/// errors (a render must never fail a run), so this writer stashes the first
/// one in a shared slot; the composition root drains it for the record's
/// single warning, so a `log` that has gone unwritable is not silently
/// truncated.
struct LogWriter<W: Write> {
    inner: W,
    failure: Arc<Mutex<Option<String>>>,
}

impl<W: Write> LogWriter<W> {
    fn record(&self, error: &io::Error) {
        let mut slot = self.failure.lock().unwrap();
        if slot.is_none() {
            *slot = Some(error.to_string());
        }
    }
}

impl<W: Write> Write for LogWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.write(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                self.record(&e);
                Err(e)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.inner.flush() {
            Ok(()) => Ok(()),
            Err(e) => {
                self.record(&e);
                Err(e)
            }
        }
    }
}

/// The record as a sink: it ingests the structured readiness and ask facts and
/// keeps `run.json` current, while the record's `log` stream is written by
/// [`RunRecord::log_renderer`]. A write failure cannot be returned through the
/// event path (it must never abort a run), so it is stashed for the
/// composition root's single warning.
impl EventSink for RunRecord {
    fn emit(&self, _label: &str, event: SessionEvent) {
        let result = match event {
            SessionEvent::SessionReady {
                label,
                agent,
                command,
                args,
                env_keys,
                acp_id,
            } => self.session_ready(SessionRecord {
                label,
                agent,
                command,
                args,
                env_keys,
                acp_id,
            }),
            SessionEvent::AskRequested { prompt, details } => {
                self.ask_requested(&prompt, details.as_deref())
            }
            SessionEvent::AskResolved { action, text } => {
                self.ask_resolved(action, text.as_deref())
            }
            _ => return,
        };
        let _ = result;
    }

    fn script_log(&self, _message: &str) {
        // `ptah.log` reaches the record through the record renderer; the
        // record model itself carries no stream.
    }
}

/// An [`EventSink`] that forwards every event to each inner sink in order —
/// the composition root's bridge from the runtime's single sink to the
/// terminal renderer, the record renderer, and the record model.
pub struct FanOut {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl FanOut {
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }
}

impl EventSink for FanOut {
    fn emit(&self, label: &str, event: SessionEvent) {
        for sink in &self.sinks {
            sink.emit(label, event.clone());
        }
    }

    fn script_log(&self, message: &str) {
        for sink in &self.sinks {
            sink.script_log(message);
        }
    }
}

/// Serialize `meta` as indented JSON terminated by a newline and replace
/// `run.json` atomically.
fn write_meta(dir: &Path, meta: &RunMeta) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(meta).map_err(io::Error::other)?;
    bytes.push(b'\n');
    atomic_write(&dir.join("run.json"), &bytes)
}

/// Write `bytes` to a sibling temp file and rename it over `path`: a reader
/// never observes a partial file, and an interrupted write leaves the previous
/// file intact.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
    }
    fs::rename(&tmp, path)
}

/// RFC 3339 in UTC (`Z`), no fractional seconds — the record's instants.
fn rfc3339(ts: Timestamp) -> String {
    format!("{ts:.0}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptah_core::config::AgentSpec;
    use std::collections::BTreeMap;

    /// The fixed start instant the id/UTC tests pin.
    fn start() -> Timestamp {
        "2026-09-12T14:32:24Z".parse().unwrap()
    }

    fn record_in(dir: &Path) -> RunRecord {
        RunRecord::create_at(dir, Path::new("main.luau"), &["ptah".into(), "run".into()], start())
            .unwrap()
    }

    fn session(label: &str, acp_id: &str) -> SessionRecord {
        SessionRecord {
            label: label.into(),
            agent: "mock".into(),
            command: "mock-agent".into(),
            args: vec!["--flag".into()],
            env_keys: vec!["TOKEN".into()],
            acp_id: acp_id.into(),
        }
    }

    // -- 2.2 run-id minting -------------------------------------------------

    #[test]
    fn run_id_shape_is_timestamp_dash_decimal() {
        let tmp = tempfile::tempdir().unwrap();
        let id = RunId::mint(start(), tmp.path()).unwrap();
        let s = id.as_str();
        let suffix = s.strip_prefix("20260912143224-").expect("timestamp prefix");
        assert_eq!(suffix.len(), 4, "{s}");
        assert!(suffix.bytes().all(|b| b.is_ascii_digit()), "{s}");
    }

    #[test]
    fn run_id_encodes_utc_regardless_of_local_zone() {
        let tmp = tempfile::tempdir().unwrap();
        // 21:32:24 at UTC+07:00 is 14:32:24 UTC; the prefix comes from the
        // absolute instant, so a non-UTC local zone cannot move it.
        let ts: Timestamp = "2026-09-12T21:32:24+07:00".parse().unwrap();
        let id = RunId::mint(ts, tmp.path()).unwrap();
        assert!(id.as_str().starts_with("20260912143224-"), "{id}");
    }

    #[test]
    fn run_ids_sort_in_start_order_across_seconds() {
        let tmp = tempfile::tempdir().unwrap();
        let first = RunId::mint("2026-09-12T14:32:24Z".parse().unwrap(), tmp.path()).unwrap();
        let second = RunId::mint("2026-09-12T14:32:25Z".parse().unwrap(), tmp.path()).unwrap();
        assert!(first.as_str() < second.as_str(), "{first} !< {second}");
    }

    #[test]
    fn occupied_run_id_is_not_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let first = RunId::mint(start(), tmp.path()).unwrap();
        fs::create_dir(tmp.path().join(first.as_str())).unwrap();
        let second = RunId::mint(start(), tmp.path()).unwrap();
        assert_ne!(first.as_str(), second.as_str());
    }

    // -- 2.3 project-root resolution ---------------------------------------

    #[test]
    fn project_root_finds_nearest_ptah_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        fs::create_dir_all(root.join(".ptah")).unwrap();
        let deep = root.join("sub").join("dir");
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(project_root(&deep), root);
        assert_eq!(project_root(&root), root);
    }

    #[test]
    fn project_root_skips_intermediate_dirs_without_ptah() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        fs::create_dir_all(root.join(".ptah")).unwrap();
        let intermediate = root.join("a").join("b");
        fs::create_dir_all(&intermediate).unwrap();
        // `a/b` has no `.ptah`; the search must keep walking up to `project`.
        assert_eq!(project_root(&intermediate), root);
    }

    #[test]
    fn project_root_falls_back_to_invocation_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let lonely = tmp.path().join("lonely");
        fs::create_dir_all(&lonely).unwrap();
        assert_eq!(project_root(&lonely), lonely);
    }

    // -- 2.4 record creation ------------------------------------------------

    #[test]
    fn first_run_creates_record_dir_and_ignore_file() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        let ignore = tmp.path().join(".ptah/runs/.gitignore");
        assert!(ignore.is_file());
        assert!(fs::read_to_string(&ignore).unwrap().contains('*'));
        assert!(record.dir().join("run.json").is_file());
        assert_eq!(
            record.dir().parent().unwrap(),
            tmp.path().join(".ptah/runs")
        );
    }

    #[test]
    fn create_publishes_log_and_run_json_together() {
        // The record is built under a temp name and renamed into place, so
        // the id never names a directory missing either file, and no temp
        // directory survives a successful create.
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        assert!(record.dir().join("log").is_file(), "log must exist");
        assert!(
            record.dir().join("run.json").is_file(),
            "run.json must exist"
        );
        let entries: Vec<_> = fs::read_dir(tmp.path().join(".ptah/runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.iter().all(|n| !n.ends_with(".tmp")),
            "no temp directory may remain: {entries:?}"
        );
    }

    #[test]
    fn existing_ignore_file_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = tmp.path().join(".ptah/runs");
        fs::create_dir_all(&runs).unwrap();
        fs::write(runs.join(".gitignore"), "custom\n").unwrap();
        let _record = record_in(tmp.path());
        assert_eq!(
            fs::read_to_string(runs.join(".gitignore")).unwrap(),
            "custom\n"
        );
    }

    #[test]
    fn unwritable_ignore_file_leaves_no_record_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = tmp.path().join(".ptah/runs");
        fs::create_dir_all(&runs).unwrap();
        // A directory occupies the ignore-file path: it cannot be written as
        // a file, so the record must not be created at all.
        fs::create_dir(runs.join(".gitignore")).unwrap();
        assert!(record_attempt(tmp.path()).is_err());
        let entries: Vec<_> = fs::read_dir(&runs)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "only the ignore path may remain: {entries:?}");
    }

    #[test]
    fn unwritable_project_dir_leaves_no_record_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // `.ptah` is a file, so `.ptah/runs` cannot be created.
        fs::write(tmp.path().join(".ptah"), "not a directory").unwrap();
        assert!(record_attempt(tmp.path()).is_err());
    }

    fn record_attempt(dir: &Path) -> io::Result<RunRecord> {
        RunRecord::create_at(dir, Path::new("main.luau"), &[], start())
    }

    // -- 2.5 log writer -----------------------------------------------------

    #[test]
    fn record_log_renderer_supersedes_quiet_and_drops_color() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        let terminal = RenderOptions {
            quiet: true,
            no_color: false,
            verbose: true,
            agent_stderr: false,
        };
        let log = record.log_renderer(terminal).unwrap();
        log.line("mock/s1", "hello from a quiet terminal");
        log.lifecycle("lifecycle reaches the file");
        let text = fs::read_to_string(record.dir().join("log")).unwrap();
        assert!(text.contains("hello from a quiet terminal"), "{text}");
        assert!(text.contains("lifecycle reaches the file"), "{text}");
        assert!(!text.contains('\u{1b}'), "no ANSI escapes: {text:?}");
    }

    /// A writer whose every write fails, for the log-failure path.
    struct AlwaysFails;

    impl Write for AlwaysFails {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn log_write_failure_is_stashed_once() {
        let cell = Arc::new(Mutex::new(None));
        let mut writer = LogWriter {
            inner: AlwaysFails,
            failure: cell.clone(),
        };
        assert!(writer.write_all(b"one").is_err());
        assert!(writer.write_all(b"two").is_err());
        assert_eq!(cell.lock().unwrap().as_deref(), Some("disk full"));
    }

    #[test]
    fn take_failure_reports_a_log_write_failure_once() {
        // A `log` that has gone unwritable must reach the composition root's
        // single warning, just like a `run.json` write failure.
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        *record.log_failure.lock().unwrap() = Some("disk full".into());
        assert_eq!(record.take_failure().as_deref(), Some("disk full"));
        assert!(record.take_failure().is_none(), "taken exactly once");
    }

    // -- 2.6 fan-out sink ---------------------------------------------------

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<String>>);

    impl EventSink for RecordingSink {
        fn emit(&self, label: &str, event: SessionEvent) {
            self.0.lock().unwrap().push(format!("emit:{label}:{event:?}"));
        }
        fn script_log(&self, message: &str) {
            self.0.lock().unwrap().push(format!("log:{message}"));
        }
    }

    #[test]
    fn fan_out_forwards_to_every_sink_in_order() {
        let a = Arc::new(RecordingSink::default());
        let b = Arc::new(RecordingSink::default());
        let sinks: Vec<Arc<dyn EventSink>> = vec![a.clone(), b.clone()];
        let fan = FanOut::new(sinks);
        fan.emit("mock/s1", SessionEvent::Prompt { text: "hi".into() });
        fan.emit("mock/s1", SessionEvent::TurnEnd);
        fan.script_log("note");
        let a_seq = a.0.lock().unwrap().clone();
        let b_seq = b.0.lock().unwrap().clone();
        assert_eq!(a_seq, b_seq);
        assert_eq!(a_seq.len(), 3);
        assert!(a_seq[0].starts_with("emit:mock/s1:"), "{a_seq:?}");
        assert_eq!(a_seq[2], "log:note");
    }

    // -- 2.7 run.json model -------------------------------------------------

    fn fully_populated() -> RunMeta {
        RunMeta {
            schema_version: SCHEMA_VERSION,
            run_id: "20260912143224-4821".into(),
            script: "main.luau".into(),
            argv: vec!["ptah".into(), "run".into(), "main.luau".into()],
            invocation_dir: "/work/project".into(),
            ptah_version: "0.1.0".into(),
            started_at: "2026-09-12T14:32:24Z".into(),
            ended_at: Some("2026-09-12T14:33:01Z".into()),
            status: RunStatus::Ok,
            exit_code: Some(0),
            error: Some("boom".into()),
            sessions: vec![session("mock/s1", "acp-1")],
            asks: vec![AskRecord {
                ordinal: 1,
                prompt: "Proceed?".into(),
                details: Some("because".into()),
                action: Some("respond".into()),
                text: Some("ship it".into()),
            }],
        }
    }

    #[test]
    fn run_meta_round_trips() {
        let meta = fully_populated();
        let json = serde_json::to_string_pretty(&meta).unwrap();
        let back: RunMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
    }

    #[test]
    fn status_derives_from_cancellation_and_exit_code() {
        let cases = [
            (0, false, RunStatus::Ok),
            (3, false, RunStatus::Failed),
            (130, false, RunStatus::Failed),
            (130, true, RunStatus::Cancelled),
        ];
        for (code, cancelled, want) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let record = record_in(tmp.path());
            record
                .finish(&RunEnd {
                    code,
                    error: None,
                    undelivered_errors: Vec::new(),
                    cancelled,
                })
                .unwrap();
            assert_eq!(
                record.meta().status,
                want,
                "code {code}, cancelled {cancelled}"
            );
            assert_eq!(record.meta().exit_code, Some(code));
        }
    }

    #[test]
    fn undelivered_task_error_is_carried_as_the_error_text() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        record
            .finish(&RunEnd {
                code: 1,
                error: None,
                undelivered_errors: vec!["task 2 failed".into()],
                cancelled: false,
            })
            .unwrap();
        let meta = record.meta();
        assert_eq!(meta.status, RunStatus::Failed);
        assert_eq!(meta.error.as_deref(), Some("task 2 failed"));
    }

    #[test]
    fn run_json_is_indented_and_newline_terminated() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        let text = fs::read_to_string(record.dir().join("run.json")).unwrap();
        assert!(text.ends_with('\n'), "{text:?}");
        assert!(text.contains("\n  \"schema_version\""), "{text}");
        assert!(serde_json::from_str::<RunMeta>(&text).is_ok());
    }

    #[test]
    fn start_write_has_running_status_and_no_end() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        let meta = record.meta();
        assert_eq!(meta.status, RunStatus::Running);
        assert!(meta.ended_at.is_none());
        assert!(meta.exit_code.is_none());
        assert_eq!(meta.started_at, "2026-09-12T14:32:24Z");
    }

    #[test]
    fn asks_are_ordinaled_in_issue_order_and_resolved_last_open() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        record.ask_requested("first?", None).unwrap();
        record.ask_requested("second?", Some("details")).unwrap();
        record
            .ask_resolved(AskAction::Respond, Some("ship it"))
            .unwrap();
        let meta = record.meta();
        assert_eq!(meta.asks[0].ordinal, 1);
        assert!(meta.asks[0].action.is_none(), "first ask stays open");
        assert_eq!(meta.asks[1].ordinal, 2);
        assert_eq!(meta.asks[1].details.as_deref(), Some("details"));
        assert_eq!(meta.asks[1].action.as_deref(), Some("respond"));
        assert_eq!(meta.asks[1].text.as_deref(), Some("ship it"));
    }

    #[test]
    fn sink_ingests_readiness_and_asks_from_structured_events() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        record.emit(
            "mock/s1",
            SessionEvent::SessionReady {
                label: "mock/s1".into(),
                agent: "mock".into(),
                command: "mock-agent".into(),
                args: vec!["--flag".into()],
                env_keys: vec!["TOKEN".into()],
                acp_id: "acp-1".into(),
            },
        );
        record.emit(
            "ask 1 main.luau",
            SessionEvent::AskRequested {
                prompt: "Proceed?".into(),
                details: None,
            },
        );
        let meta = record.meta();
        assert_eq!(meta.sessions, vec![session("mock/s1", "acp-1")]);
        assert_eq!(meta.asks.len(), 1);
        assert_eq!(meta.asks[0].ordinal, 1);
    }

    // -- 2.8 atomic rewriting ----------------------------------------------

    #[test]
    fn interrupted_write_preserves_previous_run_json() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        // One fact lands successfully.
        record.session_ready(session("mock/s1", "acp-1")).unwrap();
        let before = fs::read_to_string(record.dir().join("run.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<RunMeta>(&before).unwrap().sessions.len(),
            1
        );
        // Interrupt the next write: a directory occupies the sibling temp
        // path, so the temp create fails and the rename never happens.
        fs::create_dir(record.dir().join("run.json.tmp")).unwrap();
        assert!(record.ask_requested("Proceed?", None).is_err());
        let after = fs::read_to_string(record.dir().join("run.json")).unwrap();
        let parsed: RunMeta = serde_json::from_str(&after).unwrap();
        assert_eq!(parsed.sessions.len(), 1, "previous facts survive");
        assert!(parsed.asks.is_empty(), "the interrupted fact did not land");
    }

    #[test]
    fn first_write_failure_disables_the_record() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        record.session_ready(session("mock/s1", "acp-1")).unwrap();
        // Interrupt a write: the failure disables the record and stashes
        // its reason exactly once.
        fs::create_dir(record.dir().join("run.json.tmp")).unwrap();
        assert!(record.ask_requested("Proceed?", None).is_err());
        assert!(record.take_failure().is_some());
        // Taking the reason does not re-enable the record: a later write is
        // a no-op that neither lands nor reports a second failure.
        record.ask_requested("Again?", None).unwrap();
        let parsed: RunMeta =
            serde_json::from_str(&fs::read_to_string(record.dir().join("run.json")).unwrap())
                .unwrap();
        assert!(parsed.asks.is_empty(), "disabled record must not write");
        assert!(
            record.take_failure().is_none(),
            "exactly one failure is stashed"
        );
    }

    // -- 2.9 authored shape, never secrets ---------------------------------

    #[test]
    fn authored_args_are_recorded_without_resolved_values() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        // A registry entry whose args and env template a secret. The record
        // reads the *authored* values; the interpolation result must never
        // reach the file.
        let authored = AgentSpec {
            command: "npx".into(),
            args: vec!["--key".into(), "${ANTHROPIC_API_KEY}".into()],
            env: BTreeMap::from([("TOKEN".into(), "${ANTHROPIC_API_KEY}".into())]),
        };
        let resolved = authored.interpolate(&|_| Some("sk-live-secret".to_string()));
        record
            .session_ready(SessionRecord {
                label: "claude/s1".into(),
                agent: "claude".into(),
                command: authored.command.clone(),
                args: authored.args.clone(),
                env_keys: authored.env.keys().cloned().collect(),
                acp_id: "acp-1".into(),
            })
            .unwrap();

        let json = fs::read_to_string(record.dir().join("run.json")).unwrap();
        assert!(json.contains("${ANTHROPIC_API_KEY}"), "{json}");
        assert!(json.contains("\"TOKEN\""), "{json}");
        assert!(!json.contains("sk-live-secret"), "{json}");
        assert!(!json.contains(&resolved.args[1]), "{json}");
        assert!(!json.contains(resolved.env["TOKEN"].as_str()), "{json}");
        let parsed: RunMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sessions[0].command, "npx");
        assert_eq!(
            parsed.sessions[0].args,
            vec!["--key".to_string(), "${ANTHROPIC_API_KEY}".to_string()]
        );
        assert_eq!(parsed.sessions[0].env_keys, vec!["TOKEN".to_string()]);
    }

    // -- 4.1 run-start line ------------------------------------------------

    #[test]
    fn run_start_line_renders_paths_under_the_established_rule() {
        let invocation = Path::new("/work/project");
        let home = Path::new("/home/u");
        // Under the invocation directory: relative to it.
        assert_eq!(
            run_start_line(
                Path::new("/work/project/.ptah/runs/20260912143224-4821"),
                invocation,
                Some(home),
            ),
            "run record: .ptah/runs/20260912143224-4821"
        );
        // Outside the invocation directory but under home: `~`-collapsed.
        assert_eq!(
            run_start_line(
                Path::new("/home/u/project/.ptah/runs/20260912143224-4821"),
                invocation,
                Some(home),
            ),
            "run record: ~/project/.ptah/runs/20260912143224-4821"
        );
        // Outside home too: as received. Unknown home keeps the same rule.
        assert_eq!(
            run_start_line(Path::new("/var/tmp/run"), invocation, Some(home)),
            "run record: /var/tmp/run"
        );
        assert_eq!(
            run_start_line(Path::new("/var/tmp/run"), invocation, None),
            "run record: /var/tmp/run"
        );
    }

    #[test]
    fn inline_agent_authored_shape_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let record = record_in(tmp.path());
        record
            .session_ready(SessionRecord {
                label: "inline/s1".into(),
                // Inline specs record the authored command as the agent name.
                agent: "bin".into(),
                command: "bin".into(),
                args: vec!["--key".into(), "${API_KEY}".into()],
                env_keys: vec!["API_KEY".into()],
                acp_id: "acp-9".into(),
            })
            .unwrap();
        let parsed: RunMeta =
            serde_json::from_str(&fs::read_to_string(record.dir().join("run.json")).unwrap())
                .unwrap();
        assert_eq!(parsed.sessions[0].agent, "bin");
        assert_eq!(parsed.sessions[0].command, "bin");
        assert_eq!(
            parsed.sessions[0].args,
            vec!["--key".to_string(), "${API_KEY}".to_string()]
        );
    }
}
