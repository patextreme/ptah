# Tasks: add-ptah-ask

## 1. Core: port, types, events, config model

- [ ] 1.1 Add the `AskProvider` port, `AskRequest`/`AskOutcome`/`AskError` types to `crates/ptah-core/src/ports.rs` (design D1); verify `cargo test -p ptah-core` passes and `deps_guard` needs no allowlist changes
- [ ] 1.2 Add `AskRequested { prompt, details }` and `AskResolved { action, text: Option<String> }` variants to `SessionEvent` in `crates/ptah-core/src/events.rs`; verify core tests compile/pass with the enum extended
- [ ] 1.3 Add `AskProviderKind { Stdin, None }` (FromStr with stable error naming accepted values) and `AskSection { provider }` to `crates/ptah-core/src/config/mod.rs`; widen `Registry` with `ask: Option<AskSection>` applied as `project.or(user)` wholesale in `from_layers`; verify with unit tests: project-replaces-user, user-only applies, unknown value rejected

## 2. Config adapter: parse and validate `[ask]`

- [ ] 2.1 Widen `RegistryFile`/`parse_layer` in `crates/ptah-config/src/lib.rs` to parse the top-level `[ask]` section, validating `provider` against `AskProviderKind` at parse (error names file, section, accepted values); verify `cargo test -p ptah-config` covers: valid section, unknown value fails, absent section unchanged, agent merge semantics untouched

## 3. Luau binding and runtime semantics

- [ ] 3.1 Add `interaction: InteractionMode { Provider(Arc<dyn AskProvider>), Prohibited, Unresolved }` to `RunConfig`/`RuntimeState` (`crates/ptah-luau/src/state.rs`); verify state construction tests (or compile) pass with the new field defaulted `Unresolved`
- [ ] 3.2 Implement the `ptah.ask` async binding (`crates/ptah-luau/src/bindings.rs`, design D3): usage error on missing/non-string `prompt`; per-run ask counter + `ask {n} {script_basename}` label; ask lock taken before emitting; `AskRequested`/`AskResolved` through the sink (no resolve event when dropped); prohibited/unresolved raise their distinct messages; provider `Err` maps to failure/end-of-input messages; respond/abort result table returned; verify with in-process fake-provider tests: respond shape, abort shape, usage error, all four raises distinguishable by message, two concurrent asks serialize FIFO (second prompted only after first resolves), pending ask at script end keeps the run alive, `ptah` table stays read-only with `ask` present
- [ ] 3.3 Verify teardown semantics: SIGINT-during-ask binary-level behavior is exercised in task 5.2; in-process, assert a dropped ask emits no `AskResolved` (fake sink) — test included in 3.2's suite

## 4. CLI composition root: stdin provider, selection, flags

- [ ] 4.1 Implement the stdin provider in `crates/ptah-cli` (design D4): tokio async line reader on real stdin (shared handle built once), `/abort` and TTY Ctrl-D-on-empty resolve abort, EOF on non-TTY stdin maps to `InputClosed`, TTY-ness via `libc::isatty`; verify with unit tests around a `BufReader` over a `Cursor`/pipe fixture (gesture mapping) — the full piped path is covered in task 5.2
- [ ] 4.2 Implement mode resolution in `cli.rs` (design D2): `--ask` clap flag (value parser over `AskProviderKind`, unknown → usage error exit 2) on `run` **and** `check`; `PTAH_ASK` read with invalid value → stderr error + exit 2 (discovery-failure class); precedence flag > env > project `[ask]` > user `[ask]` > auto-detect (both isatty); verify `flags_parse`-style unit tests for flag parsing plus a resolution-order unit test over injected layers/env
- [ ] 4.3 Wire the resolved `InteractionMode` into `RunConfig.interaction` for `run` (provider injected when resolved to `Stdin`; `Prohibited`/`Unresolved` carried as modes) and pass it to check/preflight (task 6 consumes it); verify a manual `--ask=none` run of an ask-free script still succeeds (no behavior change without ask call sites)

## 5. Renderer and binary-level e2e

- [ ] 5.1 Render ask events in `crates/ptah-render` (design D4): `AskRequested` → timestamped `ask {n} {script}` prompt line (collapsed/truncated under the shared visible-char budget), indented details line when present, `> ` input cue written without newline + flushed; `AskResolved` → action line without the answer text; both bypass `--quiet` (gated like `script_log`) and follow `--no-color`; verify with renderer unit tests for line shape, truncation, quiet bypass, and no-color
- [ ] 5.2 Add a piped-stdin binary e2e harness variant (`spawn()` with piped stdin+stdout, read-until-prompt-line, write answer) and tests: `--ask=stdin` answer flow returns `{action="respond"}`, `/abort` returns `{action="abort"}`, writer-closes-stdin raises the end-of-input error (script prints via `ptah.log`), SIGINT during a pending ask exits 130 with no abort delivered; verify all pass via `cargo test --test e2e` (or the suite housing them)
- [ ] 5.3 Add the session-survival test (issue headline guarantee): task A in a slow mock turn (`MOCK_DELAY_MS`), task B asks and is answered, A completes, the same session serves another prompt with unchanged child pid (process-count helper); verify the test proves the agent subprocess is never restarted across the ask

## 6. Check and preflight findings

- [ ] 6.1 Add the ask collector branch to full-moon `Collector` (`crates/ptah-check/src/lint.rs`, design D6: `ptah` + `.`ask` + call, any argument form) with `asks` collected into `ParsedFile`; verify lint unit tests: call collected, `ptah.other(...)` and aliased `local f = ptah.ask` not collected
- [ ] 6.2 Surface interaction findings in `check()` and `preflight()` using the CLI-resolved `InteractionMode`: ask sites + `Prohibited` → prohibited finding; ask sites + `Unresolved` → unresolvable finding naming `--ask`/`PTAH_ASK`/`[ask]`; no ask sites → no finding; verify with binary-level check tests: ask+`--ask=none` exits 1, ask with no overrides in a non-TTY exits 1, ask+`PTAH_ASK=stdin` exits 0, ask-free script + `none` exits 0, alias case exits 0 (residual documented)

## 7. Type definitions, init skeleton, sync guards

- [ ] 7.1 Add `AskOptions`/`AskResult` types and the `ask` field to `.ptah/ptah.d.luau` (discriminated union `{ action: "respond", text: string } | { action: "abort" }`); verify `cargo test -p ptah-check` definitions tests and the luau-lsp analyze scenarios in the type-definitions delta: action narrowing errors on the abort arm's `text`, missing-`prompt` call type-errors
- [ ] 7.2 Extend the runtime probe script with `ptah.ask` coverage (fake provider) and extend the defs-sync test expectations; verify the probe test fails if `ask` is removed from the runtime or the defs
- [ ] 7.3 Add the commented `[ask]` block to the `ptah init` config skeleton; verify the skeleton still parses as a valid empty registry (with the section commented) and init tests pass byte-for-byte

## 8. Example and documentation

- [ ] 8.1 Add `examples/ask.luau` (strict; one ask feeding a mock-agent prompt) and its `examples.rs` test behind the piped-stdin harness variant with `PTAH_ASK=stdin`; verify `cargo test --test examples example_ask` passes and `ptah check examples/ask.luau` is clean with the synthesized registry
- [ ] 8.2 Update docs in this change: README (CLI flag list incl. `--ask` on run+check and `PTAH_ASK`; namespace table + new "Asking a human" subsection; agent-registry `[ask]` docs with the wholesale layer rule; one-line amendment to the headless-permissions section; check section gains the interaction finding), `skills/ptah/SKILL.md` (API row, semantics bullet amend, registry + check sections, trigger list), and `AGENTS.md` ports paragraph (five → six funded ports, naming `AskProvider`); verify by re-reading each doc surface against the specs' scenarios

## 9. Full-suite gates

- [ ] 9.1 Run the offline gates end-to-end: `cargo test` (workspace), `stylua .` (formatted, examples included), `ptah check` over the bundled entry scripts including `examples/ask.luau`; fix fallout until clean
- [ ] 9.2 `nix flake check` passes in the sandbox (Luau StyLua gate + in-place `ptah check` pass over bundled scripts, release build)
