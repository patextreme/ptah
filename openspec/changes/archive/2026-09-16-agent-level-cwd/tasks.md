# Tasks: agent-level-cwd

## 1. Core config model

- [x] 1.1 Add `cwd: Option<String>` with `#[serde(default)]` to `AgentSpec` (`crates/ptah-core/src/config/mod.rs`), defaulting `None` in `AgentSpec::new`, and extend `AgentSpec::interpolate` to interpolate `cwd` (unset → empty, same contract as the other fields). Update the mechanically broken struct literals (`crates/ptah-luau/src/bindings.rs` authored copy + test, `crates/ptah-render/src/record.rs` test, `crates/ptah-core` config test helper; `crates/ptah-cli/tests/acp.rs` `mock_agent_spec` uses `AgentSpec::new` and needed no edit). Verify: `cargo test -p ptah-core` passes and new unit tests assert a TOML entry with `cwd` parses, an entry without it deserializes to `None`, and `interpolate` expands `${VAR}` in `cwd` (set and unset).
- [x] 1.2 Add a `ptah-config` test that a registry entry carrying `cwd` survives discovery and project/user merge unchanged (raw view keeps `${VAR}`, resolve interpolates). Verify: `cargo test -p ptah-config` passes.

## 2. Lua runtime (`ptah-luau`)

- [x] 2.1 Parse `cwd` (string) from the inline `ptah.agent({ … })` table into both the resolved and the authored spec in `crates/ptah-luau/src/bindings.rs`. Verify: `cargo test -p ptah-luau` passes and a unit test asserts the resolved spec carries the `cwd` while the readiness event's authored shape is unchanged (no cwd field added there).
- [x] 2.2 In `new_agent_factory`'s `session` closure, resolve the working directory by precedence: `opts.cwd` → the handle's resolved `cwd` (relative joined to `state.invocation_dir`) → `state.invocation_dir`; then validate the final directory (exists and is a directory) before `transport.start_session`, raising a runtime error naming the directory on failure. Verify: `cargo test -p ptah-luau` passes with unit tests covering each precedence tier, relative resolution against the invocation dir, and the missing-directory / not-a-directory errors.
- [x] 2.3 Reject an empty resolved `cwd` (an unset `${VAR}` expands to the empty string) at `session()` instead of silently joining it to the invocation directory — the fail-fast behavior `design.md` already documents. Verify: `cargo test -p ptah-luau` passes and `empty_cwd_fails_fast_instead_of_falling_back` asserts both the interpolated and the explicit-empty cases raise before any session starts. (Added during verification.)

## 3. Type definitions

- [x] 3.1 Add `cwd: string?` to `AgentSpec` in `.ptah/ptah.d.luau` (in place — the file is the source of truth; StyLua already ignores it). Verify: the `ptah types` byte-identity test still passes (`cargo test` picks it up) and `ptah check` accepts a strict-mode fixture using `cwd = "…"` in an inline spec.
- [x] 3.2 Extend the definitions probe script to exercise an inline spec with `cwd` (and a session from it) so the runtime-probe test keeps the definitions honest. Verify: the probe test (`cargo test` — definitions synchronization suite) passes.

## 4. Offline behavior coverage (mock agent)

- [x] 4.1 Add e2e coverage (`crates/ptah-cli/tests/`, `MOCK_ECHO_CWD`): inline spec `cwd` → prompt reply echoes that directory. Verify: the new test passes.
- [x] 4.2 Add e2e coverage for a registry entry with `cwd` (temp project `.ptah/config.toml`): default session lands in the entry's directory; `agent:session({ cwd = … })` overrides it. Verify: both assertions pass in one test.
- [x] 4.3 Add e2e coverage for the failure contract: a missing directory raises a catchable Lua error at `session()` naming the directory (assert via `pcall` and message match) and the mock agent records no session. Verify: the new test passes; the existing default-cwd test (no `cwd` anywhere → invocation directory) still passes unedited.
- [x] 4.4 Positively exercise the mock's `MOCK_SESSION_LOG` (found during iteration 5): the registry-cwd e2e test now sets `MOCK_SESSION_LOG` and asserts it holds both sessions' resolved cwds, so the negative missing-cwd test's "no log file" proxy is not vacuous if the mock logging regresses. Verify: `cargo test -p ptah-cli --test e2e registry_agent_cwd_defaults_and_session_overrides` passes. (Added during verification.)

## 5. Documentation

- [x] 5.1 README: add `cwd` to the registry field documentation and the `ptah.agent`/`agent:session` API table; document precedence (session option → agent `cwd` → invocation directory), the relative-to-invocation-dir rule (stated once for both sources), the fail-fast error, and a short git-worktree usage note clarifying the cwd reaches the agent as the session working directory (the subprocess spawns in ptah's cwd) and that `ptah.exec` is unaffected. Verify: README sections present; `nix build`-consumed docs unaffected.
- [x] 5.2 Update the issue thread: comment on #28 with the change link when the branch is up. Verify: comment posted (apply-phase step, not a code gate). **Satisfied by the existing comment** `https://github.com/patextreme/ptah/issues/28#issuecomment-5695529926` (links `openspec/changes/agent-level-cwd/` on the pushed `issue-28` branch).
- [x] 5.3 Reconcile in-repo docs with the shipped behavior (found during verification): `design.md`'s empty-`cwd` decision now states the explicit pre-`is_dir` rejection (a relative `""` would otherwise join to the invocation directory and pass that probe), and `skills/ptah/SKILL.md` gains `cwd` in the inline-spec shape, the precedence/fail-fast rule on the session `cwd` option, and a `cwd` line in the registry example. Verify: `openspec validate agent-level-cwd` still passes and the skill doc's `cwd` text matches README. (Added during verification.)
- [x] 5.4 Pin the empty-`cwd` fail-fast behavior in the delta spec and README (found during iteration 3): `specs/agent-sessions/spec.md`'s resolution requirement now lists the empty case and adds an "Empty cwd fails fast" scenario, and README's failure-contract sentence lists the empty case so it matches `skills/ptah/SKILL.md`. Verify: `openspec validate agent-level-cwd --strict` passes and the README/SKILL failure text agree. (Added during verification.)
- [x] 5.5 Reconcile the definitions comment with the shipped default (found during iteration 4): `.ptah/ptah.d.luau`'s `SessionOptions` doc comment now states `cwd` defaults to the agent's own `cwd` (inline spec or registry entry), else the invocation directory, matching README/SKILL. Verify: `cargo test -p ptah-cli --test cli --test types`, `stylua --check .`, and the `luau-lsp analyze` gate over examples/fixtures all pass. (Added during verification.)

## 6. Gates

- [x] 6.1 `stylua .` clean (definitions file exempt via `.styluaignore`) and `cargo test` green workspace-wide. Verify: both commands exit 0.
- [x] 6.2 `cargo test --test examples` still passes (bundled examples gain no `cwd` usage in this change) and the in-repo `ptah check` gate stays clean. Verify: `cargo test --test examples` and a `ptah check` pass over the bundled entry scripts exit 0.
