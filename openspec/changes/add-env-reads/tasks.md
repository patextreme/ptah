# Tasks: add-env-reads

## 1. Runtime capability

- [ ] 1.1 Add an env-snapshot field to `RunConfig` and `RuntimeState`
  (`crates/ptah-luau/src/state.rs`, `BTreeMap<String, String>`, default
  empty for tests), and capture `std::env::vars().collect()` into it at
  the composition root (`crates/ptah-cli/src/cli.rs`, where
  `RunConfig`/`process_runner` are assembled). Verify:
  `cargo build -p ptah-luau -p ptah-cli` succeeds and no `std::env` call
  exists in `crates/ptah-luau/src/bindings.rs` or `sandbox.rs`.
- [ ] 1.2 Bind `os.getenv(name) -> string?` in the rebuilt `os` table
  (`crates/ptah-luau/src/sandbox.rs`, next to `time`/`clock`): closure
  over the snapshot, `nil` for missing keys, Lua error when the argument
  is missing or not a string. Verify with new unit tests in ptah-luau
  covering: set variable returns value, unset returns `nil`, set-to-empty
  returns `""`, `os.getenv()` and `os.getenv(42)` raise, `os.setenv` is
  `nil`, and repeated reads are stable.
- [ ] 1.3 Confirm `ptah-core` is untouched (`git diff --stat` shows no
  `crates/ptah-core` files) and `cargo test -p ptah-core` still passes
  including `deps_guard`.

## 2. Types and static surface

- [ ] 2.1 Extend `declare os` in `.ptah/ptah.d.luau` with
  `getenv: (name: string) -> string?`. Verify: `cargo test -p ptah-check`
  (defs embed) and the mirror invariant — a script calling `os.getenv`
  passes `ptah check`, a script calling `os.date` still fails it.
- [ ] 2.2 Extend the runtime probe test (spec-pinned by type-definitions
  "Definitions stay synchronized") to exercise `os.getenv` — a set
  variable, an unset variable, and an empty-string variable. Verify: the
  probe test fails if the defs entry is removed.

## 3. Docs and examples

- [ ] 3.1 Update `README.md`: sandbox enumeration reads
  `os.time`, `os.clock`, `os.getenv`, and `print`; add snapshot + read-only
  semantics and a clause on the trusted-scripts note that environment
  values (including secrets) are script-readable. Verify: README grep for
  `os.getenv` in the sandbox section; no `ptah.env` anywhere.
- [ ] 3.2 Add `os.getenv` to the env-reading guidance in
  `skills/ptah/SKILL.md` (in-repo canonical copy). Verify: skill doc
  mentions `os.getenv` and never `ptah.env`.
- [ ] 3.3 Add `examples/env.luau` demonstrating the nil-default pattern
  (`os.getenv("X") or "fallback"`, one screen) and register its test
  function in `crates/ptah-cli/tests/examples.rs`, setting a
  `PTAH_EXAMPLE_*` variable for the run. Verify:
  `cargo test --test examples env` passes against the mock agent.
- [ ] 3.4 Confirm the CONTEXT.md "ptah's environment" glossary entry
  (added during the design session) reads correctly alongside the shipped
  behavior; adjust wording only if implementation revealed a mismatch.

## 4. Verification

- [ ] 4.1 Full suite green: `cargo test` (workspace) — including the new
  unit tests, probe, and examples entry.
- [ ] 4.2 `openspec validate add-env-reads --strict` passes, and the
  delta scenarios (set/unset/empty/argument-error/no-mutation-surface)
  each map to a test added in 1.2, 2.2, or 3.3.
