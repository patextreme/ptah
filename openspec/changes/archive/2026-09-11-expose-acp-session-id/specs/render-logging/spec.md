# Render Logging Capability Delta

## ADDED Requirements

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
