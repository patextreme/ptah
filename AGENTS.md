# AGENTS.md

Rust CLI embedding a sandboxed Luau runtime that drives ACP-speaking agents
(Claude Code, Gemini CLI, …) over stdio. A cargo workspace of eight crates
behind one user-facing package: the `ptah` CLI plus `mock-agent` (test
fixture only — never part of the CLI surface). API/behavior details are in
`README.md`; don't duplicate them here.

## Commands

- `nix develop` — the only source of the toolchain on this machine (no system
  rustc, no rustup). All Rust work happens inside the dev shell.
- `cargo build` / `cargo test` — plain cargo works inside the dev shell.
- Single suite/filter: `cargo test --test e2e`, `cargo test --test acp <name>`.
- `stylua .` — format Luau from the repo root (StyLua defaults; the check
  gate enforces them, `.styluaignore` exempts the generated definitions).
- `ptah check <script>` — run on every `.luau` file you edit.
- `nix build` — release build via crane.
- `nix flake check` — full suite in the sandbox.

Toolchain is a **pinned nightly** in `rust-toolchain.toml` (with rustfmt,
clippy, rust-analyzer); the Nix oxalica overlay derives its toolchain from
the same pin. Don't update the pin casually.

## Testing

- The suite is **fully offline**: tests never spawn real agents or touch the
  network. Integration tests (`crates/ptah-cli/tests/`) drive the in-repo
  mock agent (`crates/ptah-cli/src/bin/mock-agent/`), located via
  `env!("CARGO_BIN_EXE_mock-agent")`.
- Mock behavior is scripted with env vars: `MOCK_CHUNKS`, `MOCK_HANG`,
  `MOCK_PERMISSION`, `MOCK_TOOL`, `MOCK_PLAN`, `MOCK_USAGE`, `MOCK_STDERR`,
  `MOCK_DELAY_MS`, … Need a new agent behavior in a test? Extend the mock,
  don't reach for a real agent.
- `crates/ptah-cli/tests/examples.rs` runs the bundled `examples/` scripts
  through the real binary against the mock agent. Discovery is explicit —
  adding an example means adding its test function there. The Nix check
  keeps `examples/` in the build source so these tests pass in the sandbox.
- `crates/ptah-core/tests/deps_guard.rs` self-scans core's sources for
  forbidden imports (real I/O, adapters, non-schema ACP, …) with a pinned
  allowlist of settled exceptions; it runs as part of the normal suite.
- `nix flake check` enforces both Luau gates over the repo's own scripts: the
  StyLua formatter check and an in-place `ptah check` pass (release binary,
  synthesized registry) over the bundled entry scripts.

## Architecture

Hexagonal workspace: adapters depend on core, never the reverse. The
compiler enforces the crate arrows; `crates/ptah-core/tests/deps_guard.rs`
additionally pins core's I/O-freedom and adapter-freedom (a self-scan
for forbidden imports with a small, commented allowlist of settled
exceptions).

- `crates/ptah-cli` — composition root **and** permanent `ptah` facade:
  a flat `pub use` of the member crates (the package's public surface,
  not a transitional shim) plus the compat re-exports `ptah::config`/
  `ptah::task`. The only crate allowed to see every member; adapter
  selection (ACP stdio transport behind `AgentTransport`) is composed
  here. Owns the binaries: `ptah` (`crates/ptah-cli/src/cli.rs` — clap CLI; exit codes
  are a contract: `0` success, `1` script error, never-observed task
  error, or (for `check`/`run` pre-flight) findings, `2` usage — and for
  `ptah check` also "check could not run" (missing script, registry
  discovery failure, luau-lsp absent) — `n` via `ptah.exit(n)`, or
  `130`/`143` when the run is cancelled by SIGINT/SIGTERM (teardown
  first), plus
  the hidden `crates/ptah-cli/src/bridge.rs` MCP result server) and `mock-agent`
  (`crates/ptah-cli/src/bin/mock-agent/`).
- `crates/ptah-acp` — ACP client over stdio. ptah declares exactly one
  client capability — the non-interactive `session.configOptions` (with
  its `boolean` sub-capability). `session/request_permission` is answered
  headless allow-all (first `AllowAlways`, else the first offered allow
  option — README has the contract); every other agent→client request
  gets method-not-found so turns never hang. Per-session config option
  state lives in the driver (captured at `session/new`, folded from
  agent pushes and `setConfig`), serialized with turns via the turn lock.
- `crates/ptah-luau` — mlua (Luau) runtime. Scripts run sandboxed (no
  I/O, network, debug); `require` resolves `.luau` modules relative to
  the requiring file; requires may traverse outside the entry script's
  directory (no tree boundary). One deviation: a `coroutine`
  table with only `yield` stays visible because the async runtime needs
  it.
- `crates/ptah-check` — `ptah check` pipeline (compile pass via mlua,
  full-moon static lints over the literal require graph, luau-lsp
  analyze with the embedded definitions) and the `run` pre-flight.
  Zero-execution: nothing here may call a compiled chunk.
- `crates/ptah-render` — streaming output renderer (color/quiet/
  verbose modes).
- `crates/ptah-config` — TOML agent registry: discovery and parse, the
  only `ConfigSource` impl. Project `.ptah/config.toml` (found upward
  from the invocation dir) overrides `~/.config/ptah/config.toml`
  **per agent name**; `${VAR}` interpolates from ptah's env at resolve
  time.
- `crates/ptah-result` — the Unix-socket result channel and the
  submit/verdict wire protocol, both halves.
- `crates/ptah-core` — the domain, I/O-free and adapter-free: task
  bookkeeping (`ptah.spawn`/`join`/`map`), turn/tool fold semantics,
  result contracts, the config model, structured `SessionEvent`s, and
  the ports. Exactly six funded ports (`crates/ptah-core/src/ports.rs`):
  `AgentTransport`, `ConfigSource`, `EventSink`, `InteractionPolicy`,
  `ProcessRunner` (funding `ptah.exec`, added deliberately via its own
  change), and `AskProvider` (funding `ptah.ask`, likewise). The set is
  closed — a new port is a design decision that gets its own change,
  not a drive-by.
- TUI readiness: core emits structured `SessionEvent`s through the
  `EventSink` port and all interaction flows through
  `InteractionPolicy` (today the headless allow-all policy), so a
  terminal UI would be another adapter, not a restructure — no current
  plan to build one.
- `skills/` — canonical skill docs (`skills/ptah/SKILL.md`) that
  consumers download and copy into their own agent setup; deployed
  copies (e.g. `~/.pi/agent/skills/ptah`) are read-only symlinks into
  the nix store. Skill-doc changes are in-repo edits inside the change's
  edit roots, never out-of-repo follow-ups.

## Workflow: OpenSpec

This repo uses spec-driven development (`openspec/`). Changes live in
`openspec/changes/<id>/` (proposal.md, design.md, tasks.md, specs/). Use the
`.pi/skills/openspec-*` skills for the propose → apply → verify → archive
lifecycle instead of freelancing. `openspec/specs/` holds the synced truth.

Scratch/artifact dirs `.work/`, `.pi/taskflows/`, `worktrees/` are gitignored.

## Git and PR conventions

- Commit messages and pull request titles follow [Conventional Commits](https://www.conventionalcommits.org/): `<type>[optional scope]: <imperative description>` (for example, `feat: add environment typings`). Use a concise subject without a trailing period; the PR title should describe the overall change.
