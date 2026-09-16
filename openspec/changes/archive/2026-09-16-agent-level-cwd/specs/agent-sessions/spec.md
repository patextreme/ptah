## ADDED Requirements

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
