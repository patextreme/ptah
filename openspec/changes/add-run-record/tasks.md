## 1. Structured session readiness

- [ ] 1.1 Add the structured readiness payload to `SessionEvent::Lifecycle` in `crates/ptah-core/src/events.rs`: readiness carries the session's label and its ACP session id as separate values alongside the existing formatted `message`, and the doc comment states that sinks read the id from the payload, not the wording. Verify: `cargo test -p ptah-core` passes and a new unit test asserts the variant carries both values.
- [ ] 1.2 Emit the payload from the ready site in `crates/ptah-luau/src/bindings.rs` (~line 340), where the id is already held, keeping the rendered message byte-identical. Verify: `cargo test -p ptah-luau` passes with no change to existing lifecycle-message assertions.
- [ ] 1.3 Widen the renderer's `Lifecycle` arm in `crates/ptah-render/src/lib.rs` for the new payload, rendering exactly the same line as before. Verify: `cargo test -p ptah-render` passes, and the existing verbose ready-line assertion in `crates/ptah-cli/tests/acp.rs` still passes unedited.

## 2. The record module in `ptah-render`

- [ ] 2.1 Declare the new dependencies in `crates/ptah-render/Cargo.toml` (`serde`, `serde_json`, `getrandom` from the workspace table). Verify: `cargo build -p ptah-render` succeeds and `Cargo.lock` gains no new crate.
- [ ] 2.2 Implement run-id minting: the start instant in UTC as `yyyymmddhhmmss`, `-`, and a short decimal suffix from `getrandom`; re-mint while the target directory exists. Verify: unit tests assert the shape, lexicographic ordering equal to start order, a UTC encoding under a non-UTC local zone, and that an occupied id yields a different one.
- [ ] 2.3 Implement project-root resolution: nearest ancestor containing `.ptah/`, else the invocation directory. Verify: unit tests over tempdirs cover both branches and the case where an intermediate directory has no `.ptah`.
- [ ] 2.4 Implement record creation: create the directory and `.gitignore` containing `*` as one unit, leave an existing ignore file unmodified, and leave no directory behind when either step fails. Verify: unit tests cover first-run creation, preservation of a user-authored ignore file, and an unwritable location leaving no directory.
- [ ] 2.5 Implement the log writer: a `Renderer` over the record's `log` file built from the terminal's verbosity with `quiet: false, no_color: true`. Verify: unit tests assert a record renderer still writes lines the terminal suppressed, and that its output carries no ANSI escapes.
- [ ] 2.6 Implement the fan-out sink: an `EventSink` that forwards `emit` and `script_log` to each inner sink in order. Verify: a unit test with two recording sinks asserts identical, equally ordered sequences for both.
- [ ] 2.7 Implement the `run.json` model: schema version, run id, script path, argv, invocation directory, ptah version, start/end as RFC 3339 UTC, status, exit code, error, sessions (label, agent, pre-interpolation `command`/`args`, env key names, ACP session id), and asks (ordinal, prompt, details, action, text), with `status` derived from exit code and signal as `ok`/`failed`/`cancelled`/`running`. Verify: unit tests round-trip a fully populated record, assert a non-zero explicit exit maps to `failed`, and assert the JSON is indented and newline-terminated.
- [ ] 2.8 Implement atomic rewriting (a sibling temp file renamed over `run.json`) behind an accessor the composition root drives. Verify: a unit test that interrupts a write asserts the previous file is still intact and parses.
- [ ] 2.9 Implement the secrets rule: take `command`/`args` from the configuration layer before `${VAR}` interpolation and record env key names only, never values. Verify: a unit test with an interpolated arg and an env table asserts the value appears nowhere in the serialized record.

## 3. Composition root wiring

- [ ] 3.1 In `crates/ptah-cli/src/cli.rs`, mint the run id and create the record before the renderer is constructed, build the terminal and record renderers behind the fan-out, keep the fan-out the value handed to the runtime as `Arc<dyn EventSink>`. Verify: a new integration test in `crates/ptah-cli/tests/` finds `.ptah/runs/<id>/log` and `run.json` after a run against the mock agent.
- [ ] 3.2 Drive the rewrite triggers — run start, session ready, ask requested, ask resolved, and every terminal transition including signal teardown — so each fact lands in `run.json` as it is learned. Verify: integration tests assert `run.json` after a run names its sessions with ACP ids, that a second ask is recorded with ordinal 2 and its resolution, and that a SIGINT run records `cancelled` with no resolution for a dropped ask.
- [ ] 3.3 Implement the best-effort failure path: one warning to stderr naming the reason and location, printed even under `--quiet`, with the run's exit code and behavior otherwise untouched; disable the record and warn once on a mid-run write failure. Verify: an integration test with an unwritable `.ptah` asserts the script still runs, exactly one warning prints, and the exit code is the script's own.
- [ ] 3.4 Ensure no record is created for a pre-flight failure, and that `check`, `types`, `init`, and `package` create none. Verify: integration assertions for each command.

## 4. The run-start line

- [ ] 4.1 Render one `ptah`-attributed line at run start naming the record directory, using the established path rule (relative to the invocation directory, `~` under home, otherwise as received) and the `exec_line` gating (present in every non-quiet mode, suppressed on the terminal by `--quiet`, always written to the record); emit it only after the record exists so the failure warning replaces it. Verify: a `ptah-render` unit test for the path forms, and an integration test asserting the line at default verbosity, its absence from the terminal under `--quiet`, and its presence in `log`.

## 5. Documentation

- [ ] 5.1 Add a README section describing `.ptah/runs/`: the layout, that `--quiet` suppresses the terminal and not the record, that the record is not a stdout capture (so `print` is absent and `ptah.log` is the script-side route), and a security note naming `run.json` as the most secret-bearing file ptah writes. Verify: read-through against the specs, with no contradiction between README and the `run-record` requirements.
- [ ] 5.2 Update `skills/ptah/SKILL.md` (~lines 440-447): drop the instruction to hand-embed `session:sessionId()` into `ptah.ask` details for correlation, pointing at the run record instead, while keeping the `sessionId()` method rows for scripts that genuinely use the id. Verify: `grep -n sessionId skills/ptah/SKILL.md` shows no remaining hand-embedding instruction and the method documentation is intact.

## 6. Gates

- [ ] 6.1 Run the full gates and confirm nothing regressed: `cargo test` for the whole workspace, `stylua .` (no changes expected — no `.luau` file is edited), `ptah check` over the repo's bundled scripts, and `nix flake check`. Verify: all pass, and `crates/ptah-cli/tests/deps_guard.rs` still passes with core's I/O-freedom intact (the record adapter lives in `ptah-render`).
