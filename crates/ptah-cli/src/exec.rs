//! The tokio `ProcessRunner` implementation funding `ptah.exec`:
//! spawn `/bin/sh -c` as its own process group (one kill reaches the
//! whole pipeline), capture stdout/stderr concurrently with the wait
//! (a child that fills a pipe can never deadlock the run), enforce the
//! timeout budget by SIGKILLing the group and reaping, and treat
//! *dropping* the returned future as cancellation: kill the group. This
//! is the composition-root twin of the ACP stdio transport — the two
//! places ptah's world-touching decisions live.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ptah_core::groups::ProcessGroups;
use ptah_core::ports::{ExecError, ExecOutcome, ProcessRunner};

/// The `ProcessRunner` used by the `ptah` CLI: every call spawns its
/// own child and group. Holds the run's process-group registry (when
/// injected) so the composition root's second-signal sweep can reach
/// exec children teardown has not drained yet.
pub struct TokioProcessRunner {
    groups: Option<Arc<ProcessGroups>>,
}

impl TokioProcessRunner {
    /// An untracked runner: execs spawn and tear down normally, but
    /// nothing records their pids for an outer-signal sweep (tests,
    /// embedders that install no monitor).
    pub fn new() -> Self {
        Self { groups: None }
    }

    /// A runner registering every spawned exec's group-leader pid in
    /// `groups` for the command's lifetime — the registry handle the
    /// composition-root signal monitor sweeps on the force escape.
    pub fn with_registry(groups: Arc<ProcessGroups>) -> Self {
        Self {
            groups: Some(groups),
        }
    }
}

impl Default for TokioProcessRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Kills the whole process group on drop (or on [`GroupKillGuard::kill_now`])
/// unless disarmed, and deregisters the pid from the kill registry on
/// every exit path. The child is its own group leader — spawned with
/// `process_group(0)` — so `-pid` reaches every process the command went
/// on to spawn, not just the shell.
struct GroupKillGuard {
    /// The group leader's pid — deregistration needs it even after the
    /// kill is disarmed.
    pid: u32,
    /// Whether the group still needs killing (natural exit disarms).
    armed: bool,
    /// The run's kill registry (`None`: this exec spawned untracked).
    groups: Option<Arc<ProcessGroups>>,
}

impl GroupKillGuard {
    fn new(pid: u32, groups: Option<Arc<ProcessGroups>>) -> Self {
        Self {
            pid,
            armed: true,
            groups,
        }
    }

    /// Send SIGKILL to the group and disarm (a second kill is pointless;
    /// the disarm also marks the normal-exit path).
    fn kill_now(&mut self) {
        if self.armed {
            self.armed = false;
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
            }
        }
    }

    /// The command ended on its own: there is nothing to kill.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for GroupKillGuard {
    fn drop(&mut self) {
        self.kill_now();
        // Registry bookkeeping rides every exit path: natural completion
        // disarmed the kill above, but the pid must still leave the
        // sweep set before the OS can hand it out again.
        if let Some(groups) = &self.groups {
            groups.deregister(self.pid);
        }
    }
}

/// Read a pipe to EOF as a (lossy, UTF-8) string.
async fn read_all<R: tokio::io::AsyncReadExt + Unpin>(mut r: R) -> std::io::Result<String> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

impl ProcessRunner for TokioProcessRunner {
    fn run<'a>(
        &'a self,
        cmd: &'a str,
        timeout_ms: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<ExecOutcome, ExecError>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::process::Command;

            // The child inherits ptah's environment and working
            // directory (no override options in v1); stdin is closed so
            // an interactive child reads EOF instead of hanging on — or
            // stealing — ptah's own stdin.
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg(cmd)
                .process_group(0)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| ExecError::Spawn(format!("`{cmd}`: {e}")))?;
            let pid = child.id().expect("child spawned");
            if let Some(groups) = &self.groups {
                groups.register(pid);
            }
            let mut guard = GroupKillGuard::new(pid, self.groups.clone());
            let mut stdout = child.stdout.take().expect("stdout piped");
            let mut stderr = child.stderr.take().expect("stderr piped");

            // Concurrent reads + wait: cancel-safe to drop at any point
            // (the guard performs the kill).
            let wait_and_read = async {
                let (out, err, status) =
                    tokio::join!(read_all(&mut stdout), read_all(&mut stderr), child.wait());
                (out.unwrap_or_default(), err.unwrap_or_default(), status)
            };

            let timed = match timeout_ms {
                Some(ms) => tokio::time::timeout(Duration::from_millis(ms), wait_and_read).await,
                None => Ok(wait_and_read.await),
            };

            match timed {
                Ok((stdout, stderr, status)) => {
                    guard.disarm();
                    Ok(ExecOutcome {
                        // A signal death is normalized to the shell's
                        // `128 + signal` convention, so `exit_code` is
                        // `Some` for every command that actually ran.
                        exit_code: status.ok().and_then(|s| exit_status_code(&s)),
                        stdout,
                        stderr,
                        timed_out: false,
                    })
                }
                Err(_elapsed) => {
                    // Budget spent: kill the whole group, then reap so no
                    // zombie remains before the outcome reports the
                    // timeout.
                    guard.kill_now();
                    let _ = child.wait().await;
                    Ok(ExecOutcome {
                        exit_code: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        timed_out: true,
                    })
                }
            }
        })
    }
}

/// `status.code()`, with a Unix signal death mapped to `128 + signal`.
fn exit_status_code(status: &std::process::ExitStatus) -> Option<i32> {
    status.code().or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt as _;
            status.signal().map(|sig| 128 + sig)
        }
        #[cfg(not(unix))]
        {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_pipeline_and_captures_output() {
        let out = TokioProcessRunner::new()
            .run("printf 'a\\nb' | wc -l", None)
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "1");
        assert!(!out.timed_out);
    }

    #[tokio::test]
    async fn nonzero_exit_and_stderr_are_data() {
        let out = TokioProcessRunner::new()
            .run("echo boom >&2; exit 3", None)
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(3));
        assert_eq!(out.stdout, "");
        assert_eq!(out.stderr, "boom\n");
    }

    #[tokio::test]
    async fn timeout_kills_and_reports() {
        let started = std::time::Instant::now();
        let out = TokioProcessRunner::new()
            .run("sleep 30", Some(100))
            .await
            .unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must not wait out the sleep"
        );
    }

    #[tokio::test]
    async fn signal_death_normalizes_to_shell_convention() {
        // The shell itself is killed by a signal: normalized to 128+sig
        // so `exit_code` is `Some` for every command that ran.
        let out = TokioProcessRunner::new()
            .run("kill -9 $$", None)
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(137));
    }

    #[tokio::test]
    async fn stdin_is_eof_not_the_terminal() {
        // `cat` reads stdin until EOF; with stdin nulled it exits at once.
        let started = std::time::Instant::now();
        let out = TokioProcessRunner::new()
            .run("cat", Some(5_000))
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout, "");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cat must see EOF immediately"
        );
    }

    #[tokio::test]
    async fn env_and_cwd_are_inherited() {
        // SAFETY: single-threaded test; the var is ours.
        unsafe { std::env::set_var("PTAH_EXEC_TEST_TOKEN", "tok-42") };
        let out = TokioProcessRunner::new()
            .run("printf %s \"$PTAH_EXEC_TEST_TOKEN\"; pwd", None)
            .await
            .unwrap();
        let expected = format!("tok-42{}\n", std::env::current_dir().unwrap().display());
        assert_eq!(out.stdout, expected);
    }

    #[tokio::test]
    async fn child_filling_a_pipe_does_not_deadlock() {
        // 1 MB through the pipe: concurrent reads + wait must drain it.
        let out = TokioProcessRunner::new()
            .run(
                "yes 0123456789 | head -c 1048576 >/dev/null; echo done",
                None,
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "done");
    }
}
