//! The `stdin` [`AskProvider`] implementation: reads answers as one
//! stdin line per ask, never writing to stdout (the renderer owns it —
//! display flows through the `EventSink` port; the provider only
//! reads). The gesture mapping is the v1 contract:
//!
//! - an answer line is the response text, unprocessed (terminator
//!   stripped);
//! - a line of exactly `/abort` — and, on a terminal, Ctrl-D at an
//!   empty prompt — resolves abort;
//! - EOF on a *non-terminal* stdin (the writer closed the pipe with no
//!   answer) is the end-of-input error, not a phantom abort.
//!
//! The TTY/pipe distinction is decided once, at construction
//! (`libc::isatty` on stdin) — the same world-touching composition
//! decision every other adapter makes at the root.

use std::future::Future;
use std::pin::Pin;

use ptah_core::ports::{AskError, AskOutcome, AskProvider, AskRequest};

/// The abort sentinel: an answer line of exactly `/abort`.
const ABORT_SENTINEL: &str = "/abort";

/// The `stdin` provider: one ask = one line from ptah's real stdin.
/// The handle is tokio's shared async stdin (built once at injection);
/// the internal `Mutex` serializes line reads, which the binding-level
/// ask lock already guarantees — this one just keeps `&self` honest.
pub struct StdinAskProvider {
    reader: tokio::sync::Mutex<tokio::io::BufReader<tokio::io::Stdin>>,
    /// stdin was a terminal at construction (decides the EOF gesture).
    tty: bool,
}

impl StdinAskProvider {
    /// Capture ptah's real stdin and its TTY-ness once.
    pub fn new() -> Self {
        Self {
            reader: tokio::sync::Mutex::new(tokio::io::BufReader::new(tokio::io::stdin())),
            tty: stdin_is_tty(),
        }
    }
}

impl Default for StdinAskProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AskProvider for StdinAskProvider {
    fn ask<'a>(
        &'a self,
        _request: AskRequest,
    ) -> Pin<Box<dyn Future<Output = Result<AskOutcome, AskError>> + Send + 'a>> {
        Box::pin(async move {
            let mut reader = self.reader.lock().await;
            read_ask_line(&mut reader, self.tty).await
        })
    }
}

/// Is ptah's stdin a terminal? A world-touching read, so it lives here
/// (the composition root crate), not in the port or the runtime.
pub fn stdin_is_tty() -> bool {
    // SAFETY: a plain isatty(2) probe on fd 0.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

/// Is ptah's stdout a terminal? (Auto-detection requires both stdin
/// and stdout to be terminals.)
pub fn stdout_is_tty() -> bool {
    // SAFETY: a plain isatty(2) probe on fd 1.
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
}

/// Read one ask answer from a buffered async reader and map the line to
/// the outcome per the gesture contract. Shared by the real stdin
/// provider and the test fixtures (a `BufReader` over a `Cursor`).
pub(crate) async fn read_ask_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut tokio::io::BufReader<R>,
    tty: bool,
) -> Result<AskOutcome, AskError> {
    use tokio::io::AsyncBufReadExt as _;

    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|e| AskError::Failed(format!("reading stdin: {e}")))?;
    map_gesture(n, &line, tty)
}

/// Map one completed `read_line` (bytes read + the raw line) to the ask
/// outcome. `n == 0` is EOF: the abort gesture on a terminal (Ctrl-D at
/// an empty prompt), the end-of-input error on a pipe. Any content is a
/// response (the terminator stripped; a writer closing without a
/// newline still had its line delivered).
pub(crate) fn map_gesture(n: usize, raw: &str, tty: bool) -> Result<AskOutcome, AskError> {
    if n == 0 {
        return if tty {
            Ok(AskOutcome::Abort)
        } else {
            Err(AskError::InputClosed)
        };
    }
    let text = strip_line_terminator(raw);
    if text == ABORT_SENTINEL {
        Ok(AskOutcome::Abort)
    } else {
        Ok(AskOutcome::Respond {
            text: text.to_string(),
        })
    }
}

/// Strip one trailing `\n` (and a preceding `\r`) — the line
/// terminator, nothing else; the answer text itself is unprocessed.
fn strip_line_terminator(raw: &str) -> &str {
    raw.strip_suffix('\n')
        .map_or(raw, |s| s.strip_suffix('\r').unwrap_or(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Drive the full read path over a `BufReader` on a `Cursor`
    /// fixture — exactly the machinery the real stdin provider uses,
    /// with scripted bytes instead of a pipe.
    async fn ask_over(input: &[u8], tty: bool) -> Result<AskOutcome, AskError> {
        let mut reader = tokio::io::BufReader::new(Cursor::new(input.to_vec()));
        read_ask_line(&mut reader, tty).await
    }

    fn respond(text: &str) -> Result<AskOutcome, AskError> {
        Ok(AskOutcome::Respond {
            text: text.to_string(),
        })
    }

    #[tokio::test]
    async fn line_answer_responds_with_the_line_unprocessed() {
        assert_eq!(ask_over(b"go ahead\n", false).await, respond("go ahead"));
        // Surrounding whitespace and case are the answer, not gestures.
        assert_eq!(ask_over(b"  ship  it  \n", false).await, respond("  ship  it  "));
        // A CRLF terminator is stripped whole (the terminator, not the
        // text).
        assert_eq!(ask_over(b"yes\r\n", false).await, respond("yes"));
        // The empty line is a response with empty text.
        assert_eq!(ask_over(b"\n", false).await, respond(""));
        // A writer closing without a newline still delivered its line.
        assert_eq!(ask_over(b"partial", false).await, respond("partial"));
    }

    #[tokio::test]
    async fn abort_sentinel_maps_to_abort() {
        assert_eq!(ask_over(b"/abort\n", false).await, Ok(AskOutcome::Abort));
        assert_eq!(ask_over(b"/abort\r\n", true).await, Ok(AskOutcome::Abort));
        // Only the exact line is the sentinel.
        assert_eq!(
            ask_over(b" /abort \n", false).await,
            respond(" /abort "),
            "padded sentinel is an answer"
        );
        assert_eq!(
            ask_over(b"/abort later\n", false).await,
            respond("/abort later")
        );
    }

    #[tokio::test]
    async fn eof_maps_by_ttyness() {
        // Terminal: Ctrl-D at an empty prompt is the abort gesture.
        assert_eq!(ask_over(b"", true).await, Ok(AskOutcome::Abort));
        // Pipe: the writer closed with no answer — end of input, an
        // error rather than a phantom abort.
        assert_eq!(ask_over(b"", false).await, Err(AskError::InputClosed));
    }

    #[test]
    fn gesture_mapping_is_pure_and_positioned_at_zero() {
        // The raw mapping function, no async machinery: n == 0 is the
        // only EOF signal read_line can produce.
        assert_eq!(map_gesture(0, "", true), Ok(AskOutcome::Abort));
        assert_eq!(map_gesture(0, "", false), Err(AskError::InputClosed));
        assert_eq!(
            map_gesture(9, "/abort\n\n", false),
            respond("/abort\n"),
            "the extra newline is content (one read = one line)"
        );
    }

    #[tokio::test]
    async fn consecutive_asks_share_one_buffered_reader() {
        // Two asks, two lines through one shared BufReader: the second
        // read must not lose buffered bytes (the shared handle is the
        // point under test). EOF with no bytes left is end of input.
        let mut reader = tokio::io::BufReader::new(Cursor::new(b"first\nsecond\n".to_vec()));
        assert_eq!(read_ask_line(&mut reader, false).await, respond("first"));
        assert_eq!(read_ask_line(&mut reader, false).await, respond("second"));
        assert_eq!(
            read_ask_line(&mut reader, false).await,
            Err(AskError::InputClosed)
        );
    }
}
