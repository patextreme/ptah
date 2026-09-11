# Scripting Capability Delta

## ADDED Requirements

### Requirement: Session identity methods
Session objects SHALL provide two identity methods, both callable as methods
(colon syntax) with no arguments: `label()`, returning the session's
ptah-local attribution label (`agentName/localId` — the id rendered on output
lines and used by `ptah.log`-style attribution), and `sessionId()`, returning
the agent-assigned ACP session id for that session: the id the agent itself
generated when the session was created, meaningful to the agent's own tooling
(e.g. resume commands, agent-side session listings) and unrelated to the local
label. `sessionId()` SHALL return a non-empty string for every live session;
the id exists before `session()` returns, because session creation completes
the agent handshake first. The two ids SHALL remain distinct concepts: the
label is ptah's attribution id, the ACP session id is the agent-side id.

#### Scenario: sessionId returns the agent-assigned id
- **WHEN** a session is created and the script calls `s:sessionId()`
- **THEN** a non-empty string is returned, equal to the id the agent assigned for that session (as observable against a scripted mock agent)

#### Scenario: sessionId is not the label
- **WHEN** a session is created as `ptah.agent("claude"):session({ id = "review" })` (label `claude/review`) and the script calls `s:sessionId()`
- **THEN** the returned value is neither `review` nor `claude/review`

#### Scenario: Distinct sessions have distinct agent ids
- **WHEN** two sessions are created from the same agent factory
- **THEN** their `sessionId()` return values differ, matching each session's own agent-assigned id

#### Scenario: Id is available before the first prompt
- **WHEN** a script calls `s:sessionId()` immediately after `agent:session(...)` returns, before any prompt
- **THEN** the call returns the session's ACP session id without yielding or erroring
