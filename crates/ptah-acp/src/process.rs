//! Agent subprocess management: spawn, the stderr pump, and teardown
//! kill/reap.

use std::sync::Arc;

use agent_client_protocol::AcpAgent;
use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};

use ptah_core::config::AgentSpec;
use ptah_core::events::SessionEvent;
use ptah_core::groups::ProcessGroups;
use ptah_core::ports::EventSink;

use super::SessionError;

/// One spawned agent subprocess plus its stderr pump.
pub(super) struct AgentProcess {
    pub stdin: async_process::ChildStdin,
    pub stdout: async_process::ChildStdout,
    /// OS process id of the agent subprocess.
    pub pid: u32,
    pub child: async_process::Child,
    /// The `-vv` stderr passthrough task; awaited at teardown so
    /// passthrough completes before the session is reported closed.
    pub stderr_task: tokio::task::JoinHandle<()>,
}

/// Start one agent subprocess as described by `spec` (attributed to
/// `label`) and pump its stderr to the sink line by line. The child's
/// group-leader pid is registered in `groups` (when present) the
/// moment `spawn_process` succeeds, so even a session that dies
/// before its driver stands up leaves nothing for the outer signal
/// sweep to miss.
pub(super) fn spawn(
    spec: &AgentSpec,
    authored_command: &str,
    label: &str,
    sink: Arc<dyn EventSink>,
    groups: Option<&Arc<ProcessGroups>>,
) -> Result<AgentProcess, SessionError> {
    let env = spec
        .env
        .iter()
        .map(|(k, v)| EnvVariable::new(k.clone(), v.clone()))
        .collect::<Vec<_>>();

    let server = McpServer::Stdio(
        McpServerStdio::new(label.to_string(), spec.command.clone())
            .args(spec.args.clone())
            .env(env),
    );
    let agent = AcpAgent::new(server);

    let (stdin, stdout, stderr, child) = agent
        .spawn_process()
        // Name the *authored* command, never the resolved one: the error is
        // rendered to the terminal and persisted in the run record, and a
        // resolved `${VAR}` value must reach neither.
        .map_err(|e| SessionError::Spawn(format!("`{authored_command}`: {e}")))?;
    let pid = child.id();
    if let Some(groups) = groups {
        groups.register(pid);
    }

    let stderr_label = label.to_string();
    let stderr_sink = sink;
    let stderr_task = tokio::spawn(async move {
        use futures::AsyncBufReadExt;
        use futures::StreamExt;
        let mut lines = futures::io::BufReader::new(stderr).lines();
        while let Some(Ok(line)) = lines.next().await {
            stderr_sink.emit(&stderr_label, SessionEvent::StderrLine { line });
        }
    });

    Ok(AgentProcess {
        stdin,
        stdout,
        pid,
        child,
        stderr_task,
    })
}

/// Kill the child's whole process group (agents are commonly launched via
/// `npx`-style wrappers) and reap it so no zombie remains. `groups`, when
/// present, is the kill registry the pid was registered in: the entry
/// leaves before the reap, so a registered pid is always live or an
/// unreaped zombie — never one the OS has recycled.
pub(super) async fn kill_and_reap(
    mut child: async_process::Child,
    groups: Option<&ProcessGroups>,
) {
    let pid = child.id();
    #[cfg(unix)]
    unsafe {
        // The child is its own process-group leader (spawn sets this).
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    if let Some(groups) = groups {
        groups.deregister(pid);
    }
    let _ = child.status().await; // reap
}
