## MODIFIED Requirements

### Requirement: TOML agent registry
Agent definitions SHALL be configured in TOML: a project-level `.ptah/config.toml` and a user-level `~/.config/ptah/config.toml`. Each agent entry SHALL define `command` (program path), and MAY define `args` (argument list), `env` (string-to-string map merged over the inherited environment), and `cwd` (working directory for the agent's sessions, overriding the invocation-directory default unless a session overrides it). Project entries SHALL override user entries for the same agent name; the two files otherwise merge.

#### Scenario: Project overrides user
- **WHEN** agent `claude` is defined in both user and project config
- **THEN** the project definition wins and the user definition's fields are not inherited

#### Scenario: Registry merge
- **WHEN** user config defines `claude` and project config defines `gemini`
- **THEN** both agents are resolvable by name

#### Scenario: No registry found
- **WHEN** neither config file exists and a script calls `ptah.agent("claude")`
- **THEN** a Lua error is raised naming the unresolved agent

#### Scenario: Entry with cwd
- **WHEN** an entry sets `cwd = "/home/u/repo-wt"` and a script creates a session for that agent without a `cwd` option
- **THEN** the session's working directory is `/home/u/repo-wt`

#### Scenario: Entry without cwd unchanged
- **WHEN** a registry file's entries define no `cwd` key
- **THEN** they parse exactly as before and sessions default to the invocation directory

### Requirement: Environment variable interpolation
String values in agent registry entries SHALL support `${VAR}` interpolation from ptah's environment at resolve time. Unset variables SHALL expand to the empty string.

#### Scenario: Store-path command
- **WHEN** `command = "${HOME}/.local/bin/claude-acp"` and `HOME=/home/pat`
- **THEN** the resolved command is `/home/pat/.local/bin/claude-acp`

#### Scenario: Unset variable
- **WHEN** `args = ["--key=${MISSING_KEY}"]` and `MISSING_KEY` is unset
- **THEN** the argument resolves to `--key=`

#### Scenario: Interpolated cwd
- **WHEN** `cwd = "${WORKTREE}"` and `WORKTREE=/home/u/repo-wt`
- **THEN** the agent's sessions use the working directory `/home/u/repo-wt` (unless a session overrides it)

### Requirement: Inline spec override
`ptah.agent(...)` SHALL accept either a registry name (string) or an inline spec table (`{ command = ..., args = ..., env = ..., cwd = ... }`) that bypasses the registry entirely.

#### Scenario: Inline spec
- **WHEN** a script calls `ptah.agent({ command = "npx", args = {"-y", "@agentclientprotocol/codex-acp"} })`
- **THEN** the resulting sessions spawn that command directly, with no registry lookup

#### Scenario: Inline spec with cwd
- **WHEN** a script calls `ptah.agent({ command = "npx", args = {"-y", "@agentclientprotocol/claude-agent-acp"}, cwd = "/path/to/worktree" })`
- **THEN** the resulting sessions use that working directory unless the session options override it
