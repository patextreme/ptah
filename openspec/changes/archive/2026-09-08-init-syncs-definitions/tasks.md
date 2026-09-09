## 1. Runtime sync logic

- [x] 1.1 In `crates/ptah-cli/src/cli.rs`, add `parse_defs_header(line: &str) -> Option<String>` matching the emitted header shape exactly (`-- ptah <version> type definitions`, non-empty version) and a `sync_definitions()` helper that reads the existing `.ptah/ptah.d.luau` (absent → `created:`), compares against both accepted forms — `definitions_bytes()` and `TYPE_DEFINITIONS` verbatim (the headerless arm, design D2) — and returns the message line (`up to date:` no-write / `updated: (old -> new)` via the parser / `updated:` bare) plus performs the write. Verify: unit tests in the `cli.rs` tests module cover all five outcomes (absent, both match forms, stale-with-header, differing-without-header) and the parser's strictness (exact shape, empty version, near-miss text → `None`).

- [x] 1.2 Rewire `run_init` to route the definitions entry through the helper while the config keeps its created/skipped loop unchanged; exactly one line per file either way, hints after, exit 0 on all non-I/O outcomes, exit 1 on write failure (unchanged). Verify: `cargo test -p ptah-cli --lib` and a manual `./target/debug/ptah init` twice in a scratch dir shows `created:` then `up to date:` + `skipped (exists):`.

## 2. Integration tests

- [x] 2.1 Rework `rerunning_init_reports_skips_and_is_idempotent` in `crates/ptah-cli/tests/init.rs`: second run reports `skipped (exists): .ptah/config.toml` and `up to date: .ptah/ptah.d.luau`, both files byte-identical, hints present, exit 0. Verify: `cargo test --test init rerunning`.

- [x] 2.2 New test `stale_definitions_are_updated_with_version_arrow`: pre-write `.ptah/ptah.d.luau` with a fabricated older emit (`-- ptah 0.0.1 type definitions` + differing body), run init, assert the file now equals `ptah types` stdout and the output line contains `updated` plus `(0.0.1 -> `. Verify: `cargo test --test init stale`.

- [x] 2.3 New test `modified_or_foreign_definitions_are_overwritten`: pre-write a differing file whose first line is not a ptah header (e.g. starts `--!strict`), run init, assert overwrite to current emit and an `updated:` line with no version suffix. Verify: `cargo test --test init modified`.

- [x] 2.4 New test `source_layout_definitions_report_up_to_date`: pre-write the file as `ptah types` stdout minus its first line, run init, assert bytes unchanged and `up to date:` reported (this is the repo-root scenario, design D2). Verify: `cargo test --test init source_layout`.

- [x] 2.5 Confirm the untouched suite still pins the unchanged behavior: `written_definitions_are_byte_identical_to_types_stdout`, `preexisting_config_survives_while_missing_defs_are_created`, `unwritable_target_fails_cleanly`, `fresh_init_creates_exactly_both_files_with_hints` pass without edits. Verify: `cargo test --test init`.

## 3. Documentation

- [x] 3.1 Rewrite `INIT_HINTS` in `cli.rs`: after upgrading, re-run `ptah init` to refresh the definitions; mention `ptah types > .ptah/ptah.d.luau` as the alternative. Verify: hint text appears in every init run's output (covered by existing hint assertions) and no longer teaches the old refresh-only incantation as the sole path.

- [x] 3.2 Update `README.md` editor-setup/init paragraphs: init writes or updates the definitions; re-running after an upgrade is the primary refresh path; `ptah types >` kept as the documented alternative; the "Existing files are skipped, never overwritten" sentence corrected to the split semantics (config skipped, definitions synced). Verify: read the section end-to-end; `rg -n 'never overwritten|skipped, never' README.md` returns no stale claims.

## 4. Change hygiene

- [x] 4.1 Full verification: `cargo test` green inside `nix develop`, `openspec validate init-syncs-definitions` passes, and `nix flake check` passes (the smoke check runs `ptah types` against the embedded defs; nothing there changes, but the suite must prove it). Record results in this file's checkboxes.
