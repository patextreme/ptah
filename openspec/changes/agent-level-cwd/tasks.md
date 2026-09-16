# Tasks: agent-level-cwd

## 1. Core config model

- [ ] 1.1 Add `cwd: Option<String>` with `#[serde(default)]` to `AgentSpec` (`crates/ptah-core/src/config/mod.rs`), defaulting `None` in `AgentSpec::new`, and extend `AgentSpec::interpolate` to interpolate `cwd` (unset → empty, same contract as the other fields). Update the mechanically broken struct literals (`crates/ptah-luau/src/bindings.rs` authored copy + test, `crates/ptah-cli/tests/acp.rs` `mock_agent_spec`, `crates/ptah-render/src/record.rs` test, `crates/ptah-core` config test helper). Verify: `cargo test -p ptah-core` passes and new unit tests assert a TOML entry with `cwd` parses, an entry without it deserializes to `None`, and `interpolate` expands `${VAR}` in `cwd` (set and unset).
- [ ] 1.2 Add a `ptah-config` test that a registry entry carrying `cwd` survives discovery and project/user merge unchanged (raw view keeps `${VAR}`, resolve interpolates). Verify: `cargo test -p ptah-config` passes.

## 2. Lua runtime (`ptah-luau`)

- [ ] 2.1 Parse `cwd` (string) from the inline `ptah.agent({ … })` table into both the resolved and the authored spec in `crates/ptah-luau/src/bindings.rs`. Verify: `cargo test -p ptah-luau` passes and a unit test asserts the resolved spec carries the `cwd` while the readiness event's authored shape is unchanged (no cwd field added there).
- [ ] 2.2 In `new_agent_factory`'s `session` closure, resolve the working directory by precedence: `opts.cwd` → the handle's resolved `cwd` (relative joined to `state.invocation_dir`) → `state.invocation_dir`; then validate the final directory (exists and is a directory) before `transport.start_session`, raising a runtime error naming the directory on failure. Verify: `cargo test -p ptah-luau` passes with unit tests covering each precedence tier, relative resolution against the invocation dir, and the missing-directory / not-a-directory errors.

## 3. Type definitions

- [ ] 3.1 Add `cwd: string?` to `AgentSpec` in `.ptah/ptah.d.luau` (in place — the file is the source of truth; StyLua already ignores it). Verify: the `ptah types` byte-identity test still passes (`cargo test` picks it up) and `ptah check` accepts a strict-mode fixture using `cwd = "…"` in an inline spec.
- [ ] 3.2 Extend the definitions probe script to exercise an inline spec with `cwd` (and a session from it) so the runtime-probe test keeps the definitions honest. Verify: the probe test (`cargo test` — definitions synchronization suite) passes.

## 4. Offline behavior coverage (mock agent)

- [ ] 4.1 Add e2e coverage (`crates/ptah-cli/tests/`, `MOCK_ECHO_CWD`): inline spec `cwd` → prompt reply echoes that directory. Verify: the new test passes.
- [ ] 4.2 Add e2e coverage for a registry entry with `cwd` (temp project `.ptah/config.toml`): default session lands in the entry's directory; `agent:session({ cwd = … })` overrides it. Verify: both assertions pass in one test.
- [ ] 4.3 Add e2e coverage for the failure contract: a missing directory raises a catchable Lua error at `session()` naming the directory (assert via `pcall` and message match) and the mock agent records no session. Verify: the new test passes; the existing default-cwd test (no `cwd` anywhere → invocation directory) still passes unedited.

## 5. Documentation

- [ ] 5.1 README: add `cwd` to the registry field documentation and the `ptah.agent`/`agent:session` API table; document precedence (session option → agent `cwd` → invocation directory), the relative-to-invocation-dir rule (stated once for both sources), the fail-fast error, and a short git-worktree usage note clarifying the cwd reaches the agent as the session working directory (the subprocess spawns in ptah's cwd) and that `ptah.exec` is unaffected. Verify: README sections present; `nix build`-consumed docs unaffected.
- [ ] 5.2 Update the issue thread: comment on #28 with the change link when the branch is up. Verify: comment posted (apply-phase step, not a code gate).

## 6. Gates

- [ ] 6.1 `stylua .` clean (definitions file exempt via `.styluaignore`) and `cargo test` green workspace-wide. Verify: both commands exit 0.
- [ ] 6.2 `cargo test --test examples` still passes (bundled examples gain no `cwd` usage in this change) and the in-repo `ptah check` gate stays clean. Verify: `cargo test --test examples` and a `ptah check` pass over the bundled entry scripts exit 0.
