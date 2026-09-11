# Render Logging Specification

## Purpose

Defines the observability contract of ptah's streaming stdout log: what is
rendered for each prompt turn and tool call, how long or short those lines
are, and which flags gate them. The render output is ptah's only log; this
capability is what makes a multi-agent fan-out followable.

## Requirements

### Requirement: Prompt turns render a prompt line
ptah SHALL render exactly one prompt line per prompt turn, at the moment the
prompt is sent to the agent, attributed to the sending session's label. The
line SHALL consist of the prefix `prompt: ` followed by the prompt text with
runs of whitespace collapsed to single spaces, truncated to a visible-char
budget (approximately 120 characters, shared with the tool peek budget) with
a trailing `…` marker when truncation occurs. The prompt line SHALL render
under default flags on every session and SHALL be suppressed by `--quiet`.

#### Scenario: Prompt line rendered
- **WHEN** a session is prompted with a multi-line prompt beginning `review the auth module`
- **THEN** one line like `prompt: review the auth module …` is rendered with that session's label before the turn's other output

#### Scenario: Long prompt truncated
- **WHEN** a session is prompted with prompt text longer than the visible-char budget
- **THEN** the prompt line shows the first budget's worth of collapsed text followed by `…` and no more

#### Scenario: Quiet suppresses the prompt line
- **WHEN** ptah runs with `--quiet` and a session is prompted
- **THEN** no prompt line is rendered

### Requirement: Tool lines carry an input peek
Tool call start and terminal lines SHALL append an input peek after the
title, chosen kind-aware from the tool call data, when the title does not
already contain the peek text (substring match, case-sensitive). The peek
SHALL be selected in priority order:

1. `execute` kind: the `command` or `cmd` string from the tool call's raw
   input object, when a non-empty string is present;
2. `read`, `edit`, `move`, or `search` kind (or `fetch`, `delete`): the first
   location's path, with `:line` appended when a line number is present;
3. otherwise: the raw input object serialized as compact JSON.

When no candidate is derivable, the line renders the title alone, exactly as
before. Peeks apply the same visible-char budget and `…` truncation as the
prompt line. The peek SHALL render on both the start line and the terminal
line of a tool call.

#### Scenario: Execute kind shows the command
- **WHEN** a tool call with kind `execute` and raw input `{"command": "git status"}` is announced (title `bash`)
- **THEN** the start line renders as `tool: bash git status` and the terminal line as `tool: bash git status (completed, …)`

#### Scenario: Read kind shows the location path
- **WHEN** a tool call with kind `read`, location `/home/u/repo/src/a.rs` line 12, and title `read` is announced with the session's cwd `/home/u/repo`
- **THEN** the rendered line names `tool: read src/a.rs:12`

#### Scenario: Title already contains the peek
- **WHEN** a tool call has title `git status` and an `execute` peek candidate `git status`
- **THEN** the peek is not appended; the line renders `tool: git status` with no duplication

#### Scenario: Unknown tool falls back to compact raw input
- **WHEN** a tool call has kind `other`, title `grep`, and raw input `{"pattern": "foo"}`
- **THEN** the rendered line names `tool: grep {"pattern":"foo"}`

#### Scenario: No derivable peek
- **WHEN** a tool call has title `Search files "foo"` with no raw input and no locations
- **THEN** the line renders the title alone, as before this change

### Requirement: Peek paths render session-relative
Location paths in peeks SHALL render relative to the session's cwd when the
path is under it, collapsed to `~` when under the user's home directory but
not under the session cwd, and otherwise as received.

#### Scenario: Path under session cwd
- **WHEN** a peek location is `/home/u/repo/src/a.rs` and the session's cwd is `/home/u/repo`
- **THEN** the path renders as `src/a.rs`

#### Scenario: Path outside session cwd but under home
- **WHEN** a peek location is `/home/u/notes/todo.md` and the session's cwd is `/home/u/repo`
- **THEN** the path renders as `~/notes/todo.md`

#### Scenario: Path outside home
- **WHEN** a peek location is `/tmp/build.log`
- **THEN** the path renders as `/tmp/build.log`

### Requirement: Rendered lines carry a full date timestamp
Every rendered line (session-attributed and `ptah` lines alike) SHALL be
prefixed with a local timestamp shaped `yyyy-mm-dd HH:MM:SS` (space-separated).
The date SHALL appear on every line, not as a session banner.

#### Scenario: Timestamp shape
- **WHEN** any render output line is emitted
- **THEN** the line begins with a `yyyy-mm-dd HH:MM:SS` local timestamp before the `[label]` prefix

### Requirement: Exec lines render command and outcome
The renderer SHALL render one line when an exec starts (carrying the command string) and one line when it ends (carrying the exit code and duration, or the timeout/spawn-failure marker), using the same timestamped line format as session lines but attributed so they read as script activity rather than a named session. Captured child stdout/stderr SHALL NOT be rendered. `--quiet` SHALL suppress exec lines entirely (they are session-event-like, not `ptah.log` script logs).

#### Scenario: Color mode shows both lines
- **WHEN** a script calls `ptah.exec("printf hi")` in default (color) output mode
- **THEN** the terminal shows a start line containing the command `printf hi` and an end line containing exit code 0 and a duration, interleaved at the moment each fires

#### Scenario: Quiet suppresses exec lines
- **WHEN** the same script runs with `--quiet`
- **THEN** no exec lines are printed; a `ptah.log` call from the script still prints

#### Scenario: Failed exec end line carries the code
- **WHEN** a script calls `ptah.exec("sh -c 'exit 4'")` in color mode
- **THEN** the end line shows exit code 4 (and the run continues; the failure is not a render error)

### Requirement: Ask lines render under the ask label
The renderer SHALL render ask activity attributed to a reserved `ask` pseudo-label carrying the per-run ask number and the entry script's basename (reading as script activity, like exec lines, not as a named session). An issued ask SHALL render its prompt as one line (whitespace collapsed and truncated under the same visible-char budget as prompt lines) and, when details were provided, one further indented line for the details under the same mechanics, followed by an input cue. On resolution, one line SHALL carry the ask number and the action (`respond` or `abort`); the response text SHALL NOT be re-echoed by ptah (the terminal already shows what was typed). Ask lines SHALL render even under `--quiet`: a suppressed prompt is a hung run, so asks are required interaction, not machine noise — `--quiet` governs streaming and diagnostics only. Ask lines are timestamped like every rendered line and follow `--no-color`.

#### Scenario: Prompt and details render
- **WHEN** a script calls `ptah.ask({ prompt = "Blocked: how to continue?", details = "probe output …" })`
- **THEN** two ask-attributed lines render — the prompt line, then an indented details line — followed by an input cue

#### Scenario: Quiet keeps ask lines
- **WHEN** the same ask is issued in a `--quiet` run
- **THEN** the ask lines still render (while agent streaming stays suppressed)

#### Scenario: Resolution line carries the action, not the text
- **WHEN** an ask is answered `retry`
- **THEN** a resolution line naming the ask and `respond` renders, and the answer text does not appear in any ptah-rendered line

#### Scenario: Ask attribution
- **WHEN** the run's first ask is issued by the script `main.luau`
- **THEN** its lines are attributed to `ask 1` carrying `main.luau`

### Requirement: Session-ready line names the ACP session id
ptah SHALL render exactly one lifecycle line when a session becomes ready —
after the agent subprocess has completed the ACP handshake and session
creation — attributed to the session's label and shaped
`{label}: session ready (acp {id})`, where `{id}` is that session's
agent-assigned ACP session id. The `acp` marker SHALL be part of the rendered
line (it names the ecosystem whose tooling accepts the id, distinguishing it
from ptah's own attribution). The line SHALL render only in verbose mode: it
is an operator diagnostic, suppressed in default mode and by `--quiet` alike.
Outside verbose mode the label remains the only session id that ever appears
in rendered output.

#### Scenario: Verbose run shows the ready line with the id
- **WHEN** a session becomes ready during a `--verbose` run
- **THEN** one line shaped `{label}: session ready (acp {id})` renders, carrying that session's agent-assigned ACP session id

#### Scenario: Default and quiet modes suppress the ready line
- **WHEN** a session becomes ready during a default-mode or `--quiet` run
- **THEN** no session-ready lifecycle line renders
