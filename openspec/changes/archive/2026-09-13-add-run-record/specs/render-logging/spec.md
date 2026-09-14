## ADDED Requirements

### Requirement: Run-start line names the run record

ptah SHALL render exactly one line when a run begins, attributed to the
reserved `ptah` pseudo-label (reading as runtime activity, like other
ptah-attributed lines, not as a named session), naming the run's record
directory and therefore the run id. The path SHALL render under the same rule
as other rendered paths: relative to the invocation directory when under it,
collapsed to `~` when under the user's home directory but not under the
invocation directory, and otherwise as received. The line SHALL render in
every non-quiet mode — default and verbose alike — and SHALL be suppressed on
the terminal by `--quiet`, which governs the terminal alone: the run's record
still receives the line (per the `run-record` capability's superset contract).
The line SHALL be emitted only once the record is in place, so it never names a
record that does not exist; when the record cannot be created, the warning
replaces it.

#### Scenario: Rendered at default verbosity
- **WHEN** a run starts in default output mode
- **THEN** one `ptah`-attributed line naming the record directory renders before any session output

#### Scenario: The line carries the run id
- **WHEN** the record is at `.ptah/runs/20260912143224-4821/` and the run was invoked from the project root
- **THEN** the line names `.ptah/runs/20260912143224-4821`

#### Scenario: The path renders relative
- **WHEN** the record directory lies under the invocation directory
- **THEN** the line shows the path relative to the invocation directory rather than absolute

#### Scenario: Quiet suppresses it on the terminal but not in the record
- **WHEN** the same run is made with `--quiet`
- **THEN** no start line appears on the terminal, and the line is present in the record's `log`

#### Scenario: No line when there is no record
- **WHEN** the record cannot be created
- **THEN** no run-start line renders and the record-unavailable warning takes its place

## MODIFIED Requirements

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

Readiness SHALL be reported to sinks as a dedicated structured event
(`SessionReady`) carrying the session's label, its agent name, its authored
invocation shape (pre-interpolation `command`/`args` and environment key
names), and the agent-assigned ACP session id, emitted once the session is
ready. The rendered line above SHALL be produced from that event, so a sink
that needs the id reads it from the event instead of parsing the rendered line
— rendered wording is the sink's business and is free to change. The event
SHALL be emitted regardless of output verbosity: it is a fact about the
session, not a rendering decision, and only the line is verbosity-gated.

#### Scenario: Verbose run shows the ready line with the id
- **WHEN** a session becomes ready during a `--verbose` run
- **THEN** one line shaped `{label}: session ready (acp {id})` renders, carrying that session's agent-assigned ACP session id

#### Scenario: Default and quiet modes suppress the ready line
- **WHEN** a session becomes ready during a default-mode or `--quiet` run
- **THEN** no session-ready lifecycle line renders

#### Scenario: The id is available without parsing the line
- **WHEN** a session becomes ready in any output mode
- **THEN** the emitted event carries that session's label and agent-assigned ACP session id as separate structured values

#### Scenario: The invocation shape travels with readiness
- **WHEN** a session selected by name from the registry becomes ready
- **THEN** the event carries the agent name and the authored (pre-interpolation) command, args, and environment key names

#### Scenario: Rewording the line does not move the fact
- **WHEN** the rendered readiness wording changes
- **THEN** a sink reading the event still obtains the same label and ACP session id
