//! Shared helpers for the integration test suite.
//!
//! Which helpers a given test binary uses varies by suite; unused ones
//! are expected, not drift.
#![allow(dead_code)]

/// Strip the renderer's leading `yyyy-mm-dd HH:MM:SS ` timestamp from
/// every line of captured output so assertions can target the
/// `[label] body` part. Lines without the prefix (script `print` output
/// never passes through the renderer) pass through unchanged.
pub fn strip_timestamps(output: &str) -> String {
    let mut stripped = output
        .lines()
        .map(strip_timestamp)
        .collect::<Vec<_>>()
        .join("\n");
    if output.ends_with('\n') {
        stripped.push('\n');
    }
    stripped
}

/// Strip one line's leading `yyyy-mm-dd HH:MM:SS ` prefix, if present.
pub fn strip_timestamp(line: &str) -> &str {
    let b = line.as_bytes();
    let is_ts = b.len() >= 20
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b' '
        && b[11..13].iter().all(u8::is_ascii_digit)
        && b[13] == b':'
        && b[14..16].iter().all(u8::is_ascii_digit)
        && b[16] == b':'
        && b[17..19].iter().all(u8::is_ascii_digit)
        && b[19] == b' ';
    if is_ts { &line[20..] } else { line }
}

/// Count live processes whose `/proc` cmdline contains `needle`. The
/// suite tags test sleeps with unique argv values, so a needle like
/// `"9871"` matches exactly the processes a test cares about.
pub fn count_processes(needle: &str) -> usize {
    let mut n = 0;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if raw
            .split(|b| *b == 0)
            .any(|arg| std::str::from_utf8(arg).is_ok_and(|s| s.contains(needle)))
        {
            n += 1;
        }
    }
    n
}

/// SIGKILL every live process whose `/proc` cmdline contains `needle`
/// (same match rule as [`count_processes`]). For sweeping orphans a
/// previously failed test run may have left under a stable tag, so the
/// next run's no-orphan assertion is about *its own* processes — never
/// for anything the current run still owns.
pub fn kill_processes(needle: &str) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let hit = raw
            .split(|b| *b == 0)
            .any(|arg| std::str::from_utf8(arg).is_ok_and(|s| s.contains(needle)));
        if hit
            && let Some(name) = entry.file_name().to_str()
            && let Ok(pid) = name.parse::<i32>()
        {
            // SAFETY: a plain SIGKILL to a pid we just observed.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// Poll `count_processes` until it reaches `want` (up to 5s), else
/// panic naming `what` — the suite's "no orphans" witness.
pub fn wait_for_processes(needle: &str, want: usize, what: &str) {
    for _ in 0..250 {
        if count_processes(needle) == want {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!(
        "expected {what} (count {want}) for processes tagged {needle:?}, got {}",
        count_processes(needle)
    );
}

/// Sweep stale processes a previously killed run left under a stable
/// tag, then wait for the sweep to land — the pre-run half of the
/// suite's no-orphan witnessing. Call this at the start of any test
/// asserting a stable tag (the e2e exec tags, the cli signal tags), so
/// the witness is about *its own* processes, never a leftover: a
/// SIGKILLed run skips drop-time kills and orphans its tagged sleeps.
pub fn clear_stale_tag(tag: &str) {
    kill_processes(tag);
    wait_for_processes(tag, 0, "stale tag cleared before the run");
}

/// A running `ptah` child with piped stdin (the test writes answers)
/// and piped stdout (the test reads rendered lines) — the ask-capability
/// binary harness. The renderer flushes per line, so line reads never
/// miss output; the `> ` input cue carries no newline, so the line
/// *after* it may arrive prefixed with it — substring matching stays
/// unaffected.
pub struct PipedRun {
    pub child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    transcript: Vec<String>,
}

impl PipedRun {
    /// Spawn `ptah run <args…> <script>` in `dir` with pinned HOME and
    /// no inherited PTAH_ASK.
    pub fn spawn(dir: &std::path::Path, script: &std::path::Path, args: &[&str]) -> Self {
        Self::spawn_env(dir, script, args, &[])
    }

    /// Like [`PipedRun::spawn`] plus environment entries on the child.
    pub fn spawn_env(
        dir: &std::path::Path,
        script: &std::path::Path,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Self {
        use std::process::{Command, Stdio};
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ptah"));
        cmd.arg("run")
            .args(args)
            .arg(script)
            .current_dir(dir)
            .env("HOME", dir)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("PTAH_ASK")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn ptah");
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = std::io::BufReader::new(child.stdout.take().expect("stdout piped"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            transcript: Vec::new(),
        }
    }

    /// Read lines (up to 10s) until one contains `needle`.
    pub fn wait_for(&mut self, needle: &str) {
        use std::io::BufRead as _;
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > std::time::Duration::from_secs(10) {
                panic!("timed out waiting for {needle:?}; seen:\n{}", self.all_output());
            }
            let mut line = String::new();
            match self.stdout.read_line(&mut line) {
                Ok(0) => panic!(
                    "stdout closed waiting for {needle:?}; seen:\n{}",
                    self.all_output()
                ),
                Ok(_) => {
                    let hit = line.contains(needle);
                    self.transcript.push(line);
                    if hit {
                        return;
                    }
                }
                Err(e) => panic!("read error waiting for {needle:?}: {e}"),
            }
        }
    }

    pub fn write_line(&mut self, line: &str) {
        use std::io::Write as _;
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Close our end: the child's stdin reads EOF.
    pub fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    pub fn all_output(&self) -> String {
        self.transcript.concat()
    }

    /// Drain the rest, reap the child: (exit code, full stdout, stderr).
    /// The stdin handle stays open until the child is reaped — closing
    /// it here would race a pending ask with a spurious EOF (the SIGINT
    /// test depends on the child never seeing stdin EOF).
    pub fn finish(&mut self) -> (i32, String, String) {
        use std::io::Read as _;
        let mut stderr = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        let mut rest = String::new();
        let _ = self.stdout.read_to_string(&mut rest);
        self.transcript.push(rest);
        let status = self.child.wait().expect("wait ptah");
        drop(self.stdin.take());
        (status.code().unwrap_or(-1), self.all_output(), stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_wellformed_prefixes() {
        assert_eq!(
            strip_timestamp("2026-08-25 12:34:56 [mock/s1] hi"),
            "[mock/s1] hi"
        );
        assert_eq!(strip_timestamp("raw print"), "raw print");
        // Malformed dates/times / no trailing space are left alone.
        assert_eq!(
            strip_timestamps("26-08-25 12:34:56 x"),
            "26-08-25 12:34:56 x"
        );
        assert_eq!(
            strip_timestamps("2026-08-25 12:34:56"),
            "2026-08-25 12:34:56"
        );
        assert_eq!(
            strip_timestamps("2026-08-25 12:34:5x a"),
            "2026-08-25 12:34:5x a"
        );
    }
}
