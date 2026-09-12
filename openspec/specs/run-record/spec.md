# Run Record Specification

## Purpose

Gives every `ptah run` a durable, self-describing record of what it was and how
it ended — the identity ptah mints for a run and the files it leaves behind
under the project's `.ptah/runs/` — so a headless run's audit trail no longer
depends on the operator having redirected stdout.

## Requirements

### Requirement: Every run gets an identity and a record

`ptah run` SHALL mint a run id before the script executes and SHALL create a
run record directory for it. The record SHALL live at
`<project>/.ptah/runs/<run id>/`, where `<project>` is the nearest ancestor of
the invocation directory that contains a `.ptah` directory, or the invocation
directory itself when no ancestor has one. Each record SHALL contain a `log`
file and a `run.json` file. No other ptah command (`check`, `types`, `init`,
`package`) SHALL create a run record, and a run that fails pre-flight SHALL
not create one.

#### Scenario: Record created for a run
- **WHEN** `ptah run main.luau` runs to completion in a project
- **THEN** exactly one new directory exists under `<project>/.ptah/runs/`, containing `log` and `run.json`

#### Scenario: Project root found upward
- **WHEN** `ptah run` is invoked from `<project>/sub/dir` and `<project>/.ptah` exists
- **THEN** the record is created under `<project>/.ptah/runs/` and nothing is created under `<project>/sub/dir/`

#### Scenario: No `.ptah` anywhere
- **WHEN** `ptah run` is invoked in a directory that has no `.ptah` in itself or any ancestor
- **THEN** the record is created under `<invocation directory>/.ptah/runs/`

#### Scenario: Only `run` records runs
- **WHEN** `ptah check`, `ptah types`, `ptah init`, or a `ptah package` command completes
- **THEN** no run record directory is created

#### Scenario: Pre-flight failure leaves no record
- **WHEN** a run fails pre-flight (broken literal require, unknown literal agent name, prohibited ask posture)
- **THEN** no record directory is created

### Requirement: Run ids are sortable and collision-free

The run id SHALL be the run's start instant in UTC formatted `yyyymmddhhmmss`,
followed by `-` and a short suffix of decimal digits. The timestamp prefix
SHALL sort lexicographically in the order the runs started, in every timezone;
runs that start within the same second share that prefix, and their relative
order is unspecified. When a directory for a freshly minted id already exists,
ptah SHALL mint another id rather than reuse, merge into, or overwrite that
directory.

#### Scenario: Id shape
- **WHEN** a run starts at 2026-09-12T14:32:24Z
- **THEN** its id begins `20260912143224-` and the remainder is decimal digits

#### Scenario: Ordering reflects start order across seconds
- **WHEN** two runs start in different seconds
- **THEN** ascending lexicographic order of their ids is the order they started

#### Scenario: Same-second runs share a prefix
- **WHEN** two runs start within the same second
- **THEN** their ids share the timestamp prefix and their relative sort order is unspecified

#### Scenario: UTC regardless of the machine's zone
- **WHEN** a run starts at local time 2026-09-12 21:32:24 in a UTC+7 zone
- **THEN** its id encodes `20260912143224`, the UTC instant

#### Scenario: An occupied id is not reused
- **WHEN** the directory for a freshly minted id already exists
- **THEN** the record is written under a different id and the pre-existing directory is left untouched

### Requirement: The record self-ignores

Creating the record SHALL create `.ptah/runs/.gitignore` containing `*` if it
does not already exist, and ptah SHALL NOT modify an existing one. The record
directory and that ignore file SHALL be created as one unit: ptah SHALL NOT
leave behind a record directory that lacks it.

#### Scenario: First run creates the ignore file
- **WHEN** a run creates `.ptah/runs/` for the first time
- **THEN** `.ptah/runs/.gitignore` exists and contains `*`

#### Scenario: Records stay out of commits
- **WHEN** a project's `.ptah` directory is tracked by git and a run has completed
- **THEN** `git add -A` stages no file from `.ptah/runs/`

#### Scenario: An existing ignore file is preserved
- **WHEN** `.ptah/runs/.gitignore` already exists with different content
- **THEN** it is left unmodified

#### Scenario: No record without its ignore file
- **WHEN** `.ptah/runs/.gitignore` cannot be written
- **THEN** no record directory is left behind for that run

### Requirement: The log is the rendered stream, a superset of the terminal

The `log` file SHALL contain ptah's rendered output for the run: every line the
renderer produces at the run's verbosity, including ask lines and `ptah.log`
lines, and including lines the terminal suppressed because `--quiet` was set.
It SHALL contain no ANSI escape sequences. It SHALL NOT contain the script's
own `print` output, and SHALL NOT contain what ptah writes to standard error
(pre-flight findings, terminal error reports). Lines SHALL be appended as they
are produced and flushed per line, so an abnormally killed run leaves every
line it had completed.

#### Scenario: Superset under quiet
- **WHEN** a script that prompts an agent runs with `--quiet`
- **THEN** the terminal shows no streaming output and the record's `log` contains the prompt and tool lines

#### Scenario: Verbose lifecycle lines reach the file
- **WHEN** the same run is made with `-v`
- **THEN** the record's `log` holds the lifecycle diagnostics alongside the streaming lines

#### Scenario: Ask lines are recorded
- **WHEN** a run answers a `ptah.ask`
- **THEN** the record's `log` carries the ask prompt line and the resolution line

#### Scenario: No color in the file
- **WHEN** a run renders colored output on the terminal
- **THEN** the record's `log` holds the equivalent lines with no ANSI escapes

#### Scenario: Script print output is excluded
- **WHEN** a script calls `print("hello")`
- **THEN** `hello` does not appear in the record's `log`

#### Scenario: Standard error is excluded
- **WHEN** a run terminates with an uncaught script error, printing it to standard error
- **THEN** that error text does not appear in the record's `log`

#### Scenario: An abnormally killed run keeps its completed lines
- **WHEN** a run is killed with SIGKILL mid-turn
- **THEN** the record's `log` ends at the last line the run had finished rendering

### Requirement: `run.json` carries the run's identity and shape

`run.json` SHALL be a JSON object containing a schema version and: the run id;
the entry script's path; the process argv; the invocation directory; the ptah
version; start and end instants as RFC 3339 in UTC; a status of `running`,
`ok`, `failed`, or `cancelled`; the process exit code once the run has ended;
the terminal error message when the run ended in error; the sessions that
started; and the asks that were issued. The status SHALL be derived from how
the process ended: `cancelled` when the runtime reports the run was terminated
by a signal, otherwise `ok` when it exited 0 and `failed` when it exited
non-zero, and `running` while it has not ended. Cancellation SHALL be taken
from the runtime's report of how the run ended, not inferred from the exit
code, so a script that exits with a signal's code (`ptah.exit(130)`) is
`failed`, not `cancelled`; a force-kill that never returns through the runtime
(a second termination signal, or SIGKILL) SHALL leave the status `running`.
The file SHALL be indented for reading and terminated with a newline.

#### Scenario: Start write
- **WHEN** a run has started and not yet ended
- **THEN** `run.json` exists with status `running`, no end instant, and no exit code

#### Scenario: Completed run
- **WHEN** a script completes normally
- **THEN** `run.json` carries status `ok`, an end instant, and exit code 0

#### Scenario: Uncaught script error
- **WHEN** a script error escapes the main chunk
- **THEN** `run.json` carries status `failed` and the error message

#### Scenario: Undelivered task error
- **WHEN** the script finishes while a spawned task's error was never observed
- **THEN** `run.json` carries status `failed` and that task's error message

#### Scenario: Cancellation
- **WHEN** the first SIGINT or SIGTERM cancels the run
- **THEN** `run.json` carries status `cancelled` and the signal's exit code

#### Scenario: Non-zero explicit exit
- **WHEN** a script calls `ptah.exit(3)`
- **THEN** `run.json` carries status `failed` and exit code 3

#### Scenario: Explicit exit with a signal's code
- **WHEN** a script calls `ptah.exit(130)`
- **THEN** `run.json` carries status `failed` and exit code 130

#### Scenario: Killed without teardown
- **WHEN** a run is force-killed with no teardown (SIGKILL, or a second SIGINT/SIGTERM)
- **THEN** `run.json` still names the script, argv, invocation directory, start instant, and status `running`

### Requirement: The record pins invocation shape, never secrets

For each session that started, `run.json` SHALL record the agent's `command`
and `args` as authored — before `${VAR}` interpolation — and the names, never
the values, of the environment keys declared for that agent. "Authored" means
the configuration file for an agent selected by name from the registry, and the
authored table for an inline `ptah.agent({ ... })` spec; the recorded agent
name is the registry name, or the authored command for an inline spec. No
resolved interpolation value, and no environment value inherited from ptah's
environment, SHALL appear anywhere in `run.json`. Only agents whose sessions
actually started SHALL appear.

#### Scenario: Environment values are absent
- **WHEN** an agent's registry entry declares `env = { ANTHROPIC_API_KEY = "${ANTHROPIC_API_KEY}" }`
- **THEN** `run.json` records the key name and no value

#### Scenario: Interpolation is not resolved
- **WHEN** an agent's registry entry declares `args = ["--key", "${ANTHROPIC_API_KEY}"]`
- **THEN** `run.json` records those args verbatim, and the key's value appears nowhere in the file

#### Scenario: Inline agent spec is recorded authored
- **WHEN** a script starts a session with `ptah.agent({ command = "bin", args = ["--key", "${API_KEY}"] })`
- **THEN** `run.json` records `command = "bin"`, the authored args verbatim, and the key's value appears nowhere

#### Scenario: Unused agents are absent
- **WHEN** the registry defines five agents and the script starts sessions for two
- **THEN** `run.json` lists exactly those two

#### Scenario: Inherited environment is not recorded
- **WHEN** an agent inherits ptah's environment
- **THEN** only the declared env key names appear

### Requirement: Sessions and asks are recorded for correlation

For each session that started, `run.json` SHALL record the session's label, the
agent name, the agent's invocation shape, and the agent-assigned ACP session
id, taken from the sessions' structured readiness events rather than parsed
from rendered lines. For each ask issued, it SHALL record the ask's ordinal,
its prompt, and its details when supplied; once the ask resolves, it SHALL
additionally record the action and the full response text. The ordinal is the
record's own emission-order count of issued asks, which matches the `ask {n}`
rendered lines carry because asks are serialized in issue order. An ask the run
was torn down while waiting on SHALL be recorded with no resolution.

#### Scenario: Session labels map to ACP ids
- **WHEN** a script creates two sessions
- **THEN** each appears in `run.json` with its label, agent name, and ACP session id

#### Scenario: Answered ask
- **WHEN** a run's second ask is answered `ship it`
- **THEN** `run.json` records an ask with ordinal 2, its prompt, action `respond`, and text `ship it`

#### Scenario: Aborted ask
- **WHEN** an ask is aborted
- **THEN** it is recorded with action `abort` and no text

#### Scenario: Ask details
- **WHEN** an ask supplies `details`
- **THEN** the details are recorded

#### Scenario: Ask dropped by teardown
- **WHEN** the run is cancelled while an ask is pending
- **THEN** the ask appears with its prompt and no resolution

### Requirement: The record is rewritten atomically as facts arrive

ptah SHALL write `run.json` at each point it learns a fact the record carries:
at run start, when a session becomes ready, when an ask is issued, when an ask
resolves, and at every run end. Each write SHALL replace the file atomically,
so no reader ever observes a partial or invalid `run.json`.

#### Scenario: Readers never see a partial file
- **WHEN** `run.json` is read at any instant during a run
- **THEN** it parses as complete JSON

#### Scenario: An interrupted write preserves the previous file
- **WHEN** a record write is interrupted
- **THEN** the previous `run.json` remains intact and still parses

#### Scenario: Sessions are recorded without waiting for the run to end
- **WHEN** a session becomes ready
- **THEN** `run.json` already names that session

#### Scenario: Run end is recorded
- **WHEN** the run ends
- **THEN** `run.json` carries the status, end instant, and exit code

### Requirement: An unwritable record never fails the run

When ptah cannot create the record, the run SHALL proceed without it, having
printed exactly one warning to standard error naming the reason and the
location. That warning SHALL be printed even under `--quiet`. When a record
write fails after the run has started, ptah SHALL disable the record, warn
once, and let the run continue. No record failure SHALL abort the script or
change the run's exit code.

#### Scenario: Read-only location
- **WHEN** `ptah run` executes where `.ptah` or `.ptah/runs/` cannot be created or written
- **THEN** the script runs normally, one warning names the location, and the exit code is the script's own

#### Scenario: The warning survives quiet
- **WHEN** the same run is made with `--quiet`
- **THEN** the warning is still printed

#### Scenario: Mid-run write failure
- **WHEN** the record's files become unwritable while the run is in progress
- **THEN** the run continues to completion and exactly one warning is printed

#### Scenario: Failure of the run is unaffected
- **WHEN** a run with an unwritable record location ends in an uncaught script error
- **THEN** the exit code reports the script error, not a record error
