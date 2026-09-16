# Agent Sessions Specification

## Purpose

Defines how ptah communicates with ACP agents as a client: subprocess lifecycle, session creation, prompt turn behavior, streaming update handling, and the capability surface ptah exposes to agents.

## Requirements

### Requirement: Agents run as ACP stdio subprocesses
ptah SHALL act as an ACP client by spawning the configured agent program as a child process and speaking JSON-RPC over its stdin/stdout, performing the `initialize` handshake before any session is created.

#### Scenario: Handshake before use
- **WHEN** a session is created for an agent
- **THEN** ptah has sent `initialize` and accepted the agent's protocol version response before sending `session/new`

#### Scenario: Spawn failure fails fast
- **WHEN** the configured agent command cannot be spawned (missing binary)
- **THEN** the `session()` call raises a Lua error naming the command

### Requirement: One subprocess per session
Each `agent:session(options)` call SHALL spawn a dedicated agent subprocess for that session; sessions never share a process. `session:close()` SHALL end the ACP session and terminate that subprocess.

#### Scenario: Independent sessions
- **WHEN** a script creates two sessions from the same agent factory
- **THEN** two agent subprocesses are running, each serving exactly one session

#### Scenario: Close reaps the process
- **WHEN** `s:close()` completes
- **THEN** the session's agent subprocess has exited and is reaped (no zombie remains)

### Requirement: Prompt turns drive the full update stream
During a prompt turn ptah SHALL render exactly one prompt line when the
prompt is sent (per the `render-logging` capability's prompt-line
contract), receive `session/update` notifications and: accumulate `agent_message_chunk` into the final message text, render tool call and plan updates for display per the tool-line contract below, and render `usage_update` context-window information for display. Token counts SHALL be taken from the `session/prompt` response and returned as the result's `usage` (zero when the response reports none). The turn SHALL complete when the `session/prompt` response arrives, returning its `stopReason`.

Tool-line contract: ptah SHALL maintain, per session, a map of tool call id to title learned from `tool_call` notifications, and SHALL resolve titles for lines derived from `tool_call_update` notifications through that map (falling back to the raw call id only when an update arrives for a call that was never announced). A tool call SHALL render at most one line when it enters `in_progress` (the start line: the title with the input peek appended per the `render-logging` capability, and no status suffix) and at most one line when it reaches a terminal status (`completed` or `failed`; the same peek appended). `pending` status SHALL NOT render, and a status transition that repeats the last-rendered status for the same call SHALL NOT render. Terminal lines SHALL include the call's elapsed wall-clock duration, measured from the call's first rendered activity (`in_progress` transition, or first observation when no `in_progress` ever arrives).

#### Scenario: Chunks assemble final text
- **WHEN** an agent streams two `agent_message_chunk` updates ("Hel", "lo") and ends its turn
- **THEN** the prompt result's `text` is `"Hello"`

#### Scenario: Usage accounting
- **WHEN** an agent reports token usage on its `session/prompt` response
- **THEN** the result's `usage` reflects the reported token counts

#### Scenario: Context-window usage rendered
- **WHEN** an agent emits a `usage_update` during a turn
- **THEN** the reported context-window usage is rendered with session attribution

#### Scenario: Tool call renders start and terminal lines
- **WHEN** an agent announces a tool call (`tool_call`, status `pending`), updates it to `in_progress`, then to `completed`
- **THEN** exactly two lines are rendered for that call: the title (with peek, when one is derivable) alone at start, and the title with peek, terminal status and elapsed duration at completion

#### Scenario: Update lines use titles, not raw call ids
- **WHEN** an agent announces a tool call titled `Search files "foo"` and then sends a `tool_call_update` changing its status
- **THEN** the rendered line names `Search files "foo"` and never the raw call id

#### Scenario: Repeated statuses do not flood the log
- **WHEN** an agent sends the same status for the same tool call more than once (e.g. two consecutive `in_progress` updates, or a re-sent terminal status)
- **THEN** each repeated status renders nothing beyond the line its first occurrence already rendered

#### Scenario: Pending is silent
- **WHEN** an agent announces a tool call with `pending` status
- **THEN** no line is rendered for the `pending` announcement

#### Scenario: Unannounced update falls back to the call id
- **WHEN** an agent sends a `tool_call_update` for a call id that was never announced by a `tool_call`
- **THEN** the rendered line falls back to the raw call id

#### Scenario: Direct completion without progress
- **WHEN** an agent announces a tool call (`pending`) and updates it straight to `completed` without any `in_progress` update
- **THEN** only the terminal line is rendered, with duration measured from first observation

### Requirement: No interactive client capabilities are exposed
ptah SHALL declare exactly one client capability during initialization: `session.configOptions` (a non-interactive capability; it commits ptah to nothing interactive). ptah runs headless, so `session/request_permission` requests SHALL be answered, on every session, for the whole session lifetime, by selecting an allow option the agent offered: the first `AllowAlways` option when one is offered, otherwise the first allow-kind option of any other kind (e.g. `AllowOnce`). When the offer contains no allow-kind option at all, ptah SHALL respond with a JSON-RPC error indicating the method is unsupported. All other agent-to-client requests ptah has declared no support for — `fs/read_text_file`, `fs/write_text_file`, `terminal/*`, `elicitation/create` — SHALL be answered with a JSON-RPC error indicating the method is unsupported, and MUST NOT block the turn indefinitely. Selecting `AllowAlways` MAY cause the agent to persist an allow rule in its own configuration beyond the ptah run. `config_option_update` is a notification (no reply exists to deny) and SHALL be consumed, not ignored.

#### Scenario: Permission request allowed
- **WHEN** an agent calls `session/request_permission` offering allow options
- **THEN** ptah responds with the first `AllowAlways` option's id and the turn continues

#### Scenario: Permission request without allow-always
- **WHEN** an agent calls `session/request_permission` offering only an `AllowOnce` option
- **THEN** ptah responds with that option's id and the turn continues

#### Scenario: Permission request denied
- **WHEN** an agent calls `session/request_permission` offering only reject options, or no options
- **THEN** ptah responds with an unsupported-method error and the turn continues

#### Scenario: Unsupported requests still rejected
- **WHEN** an agent issues `fs/read_text_file`, `fs/write_text_file`, `terminal/*`, or `elicitation/create`
- **THEN** ptah responds with an unsupported-method error

#### Scenario: No hanging turns
- **WHEN** an agent issues any agent-to-client request mid-turn
- **THEN** ptah replies promptly and the turn continues toward completion

#### Scenario: Config option updates are consumed
- **WHEN** an agent sends a `session/update` carrying a `config_option_update`
- **THEN** ptah folds the update into that session's option state (no reply exists for notifications)

### Requirement: Cancellation maps to session/cancel
Calling `session:cancel()` during an in-flight turn SHALL send the `session/cancel` notification to that session's agent, and `timeoutMs` expiry on `prompt` SHALL do the same before raising the timeout error.

#### Scenario: Timeout cancels remotely
- **WHEN** a prompt exceeds `timeoutMs`
- **THEN** ptah sends `session/cancel` and raises the timeout error to the script

### Requirement: Processes are torn down at run end
At script end (normal, error, or explicit `ptah.exit`), ptah SHALL terminate and reap every still-running agent subprocess. When the run is cancelled by an outer signal (SIGINT/SIGTERM forwarded by the composition root), teardown SHALL likewise terminate and reap every agent subprocess before the run exits with the shell-conventional 128+signal code (130/143). A second outer signal arriving while teardown is still in progress SHALL kill every not-yet-reaped agent's whole process group before the immediate exit — no agent subprocess, nor any process in its group, outlives the ptah process on any end-of-run path, including the force escape.

#### Scenario: Normal exit cleanup
- **WHEN** the script finishes with sessions left open
- **THEN** all agent subprocesses are terminated and reaped before the ptah process exits

#### Scenario: Error exit cleanup
- **WHEN** an uncaught script error aborts the run
- **THEN** in-flight turns are cancelled and all agent subprocesses are terminated and reaped

#### Scenario: Ctrl-C kills the agent and exits 130
- **WHEN** SIGINT interrupts a run while an agent session is in flight (the agent sits in its own process group, out of the terminal signal's reach)
- **THEN** teardown kills the agent's process group before the process exits, and the run exits 130 rather than orphaning the agent (SIGTERM likewise runs teardown and exits 143)

#### Scenario: Second signal kills not-yet-reaped agents
- **WHEN** a first signal starts teardown with a live agent and a second signal arrives before teardown has killed it
- **THEN** the agent's process group is killed before the immediate exit, and the exit code matches the second signal (130 for SIGINT, 143 for SIGTERM)

### Requirement: Session working directory resolution
At session creation ptah SHALL resolve the session's working directory by precedence: an explicit `cwd` in the `agent:session(...)` options wins; otherwise the agent's resolved `cwd` (from the inline spec or the registry entry, after `${VAR}` interpolation) applies; otherwise the default is ptah's invocation directory. A relative `cwd` — from the session options or the agent spec — SHALL resolve against ptah's invocation directory. A resolved working directory that is empty, does not exist, or is not a directory SHALL raise a catchable Lua error at the `session()` call — naming the directory, or explaining the empty value — before any agent subprocess spawns. `ptah.exec` is unaffected: shell steps keep inheriting ptah's process working directory.

#### Scenario: Session option overrides agent cwd
- **WHEN** an agent handle carries `cwd = "/home/u/wt-a"` and the script calls `agent:session({ cwd = "/home/u/wt-b" })`
- **THEN** the session's working directory is `/home/u/wt-b`

#### Scenario: Agent cwd applies by default
- **WHEN** an agent handle carries `cwd = "/home/u/wt-a"` and the script calls `agent:session({})` with no `cwd` option
- **THEN** the session's working directory is `/home/u/wt-a`

#### Scenario: Relative agent cwd resolves against the invocation directory
- **WHEN** `ptah run` was invoked from `/home/u/repo` and an agent spec sets `cwd = "worktrees/42"`
- **THEN** the session's working directory is `/home/u/repo/worktrees/42`

#### Scenario: Missing directory fails fast
- **WHEN** a resolved working directory (`/home/u/nope`) does not exist
- **THEN** the `session()` call raises a catchable Lua error naming `/home/u/nope`, and no agent subprocess is spawned

#### Scenario: Non-directory fails fast
- **WHEN** a resolved working directory names an existing regular file
- **THEN** the `session()` call raises a catchable Lua error naming that path, and no agent subprocess is spawned

#### Scenario: Empty cwd fails fast
- **WHEN** a resolved working directory is the empty string (e.g. a `cwd` of `${VAR}` with `VAR` unset)
- **THEN** the `session()` call raises a catchable Lua error before any agent subprocess is spawned, instead of silently falling back to the invocation directory
