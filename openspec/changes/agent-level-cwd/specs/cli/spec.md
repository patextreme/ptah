## MODIFIED Requirements

### Requirement: Session cwd defaults to invocation directory
The working directory for agent sessions SHALL be resolved at session creation (per the agent-sessions capability's resolution requirement): an explicit per-session `cwd` option wins; otherwise the agent's resolved `cwd` (inline spec or registry entry) applies; otherwise the default SHALL be the directory from which `ptah run` was invoked.

#### Scenario: Default cwd
- **WHEN** a session is created without an explicit `cwd` from an agent with no resolved `cwd`
- **THEN** the session's working directory is ptah's invocation directory

#### Scenario: Agent-level cwd
- **WHEN** a session is created without an explicit `cwd` from an agent whose spec carries `cwd = "/home/u/wt"`
- **THEN** the session's working directory is `/home/u/wt`
