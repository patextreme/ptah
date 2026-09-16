## Why

A script that receives an opaque `Agent` handle has no way to redirect where that handle's sessions run. Ptah Playbooks (`patextreme/ptah-libs`) is the concrete case: playbooks take `agent` handles in config and create sessions internally with only `{ id = …, resultSchema = … }` — there is no consumer seam to point those sessions at a git worktree. Worktree isolation is the standard shape for parallel agent workflows (one checkout per issue/PR, agents mutate it without touching the main tree), and today's only workarounds — `cd` into the worktree before `ptah run`, or a Luau wrapper table reimplemented per consumer — respectively forbid per-role placement / multi-worktree fan-out, or duplicate the same shim everywhere. (GitHub issue #28.)

## What Changes

- `AgentSpec` gains an optional `cwd: string?`:
  - inline specs: `ptah.agent({ command = …, args = …, env = …, cwd = … })`
  - registry entries: `[agents.<name>] cwd = "…"` in TOML, with `${VAR}` interpolation like `command`/`args`/`env`
- Session working-directory precedence becomes: explicit `agent:session({ cwd = … })` → the handle's resolved `cwd` → ptah's invocation directory (today's default, unchanged).
- A relative `cwd` from either source resolves relative to ptah's invocation directory — the same rule the per-session `cwd` already follows, documented once.
- A resolved `cwd` that does not exist or is not a directory raises a catchable Lua error at `session()` naming the directory (fail fast, before spawning), instead of handing the agent a bogus working directory to fail on downstream. This validation applies uniformly to every cwd source — it is a small behavior change for an explicitly wrong per-session `cwd`, which today reaches the agent unvalidated.
- The type definitions (`AgentSpec`) and the README registry/session docs reflect the field.
- Non-goals: `ptah.exec` keeps inheriting ptah's process cwd (no `cwd`/`env` override in v1); ptah does not create or remove git worktrees (callers drive `git worktree`); no per-session `env` override; session environment inheritance is unchanged.

## Capabilities

### New Capabilities

- (none)

### Modified Capabilities

- `agent-registry` — the TOML registry requirement and the inline-spec requirement gain the optional `cwd` field; the interpolation requirement's coverage is made concrete for `cwd`.
- `agent-sessions` — new requirement pinning session working-directory resolution (precedence and relative-resolution rule) and the missing-directory failure contract at `session()`.
- `cli` — the "Session cwd defaults to invocation directory" requirement gains the agent-level defaulting layer (session option wins → handle `cwd` → invocation directory).
- `type-definitions` — the `AgentSpec` type in the definitions gains `cwd: string?`.

## Impact

- `crates/ptah-core` (`config::AgentSpec` — new optional field, interpolation extended; the field is `serde(default)` so existing registry files parse unchanged).
- `crates/ptah-config` (no parse code changes expected — the field flows through the shared `AgentSpec` derive).
- `crates/ptah-luau` (inline-spec table parse; session-cwd resolution in the `agent:session` binding — precedence + validation).
- `crates/ptah-acp` (no protocol change: the session `cwd` already rides `session/new`; only its defaulting moves upstream).
- `.ptah/ptah.d.luau` (generated definitions — `AgentSpec`).
- `README.md` (registry field table, session options, worktree usage note).
- Tests: offline coverage via the mock agent's existing `MOCK_ECHO_CWD` (each prompt replies with the session's `cwd`); registry/interpolation unit tests in `ptah-core`/`ptah-config`.
