## Why

The project definitions (`.ptah/ptah.d.luau`) are a derived artifact —
byte-identical to `ptah types` output by construction — yet `ptah init`
treats them like user-authored config and skips them forever once they
exist. After upgrading ptah, the binary and its embedded definitions move
while the editor keeps type-checking against the old API; the only remedy
is a shell incantation (`ptah types > .ptah/ptah.d.luau`) documented in
the README but easy to forget. The version header on stale files was
designed to make this drift identifiable, not fixable — init can fix it.

## What Changes

- `ptah init` **syncs** the project definitions instead of skipping them:
  created when absent; **overwritten** when they differ from the current
  binary's emitted output; left untouched only when already byte-identical
  to it. Overwrite is unconditional apart from that byte-identity check —
  a hand-edited or foreign file is overwritten too, since the file is
  documented as generated output.
- The no-op check accepts two layouts: the emitted form (version header +
  body) and the headerless body verbatim. The second arm protects this
  repository's own source-of-truth `.ptah/ptah.d.luau` (a compile input
  via `include_str!`, and headerless by design) from being rewritten with
  a prepended header — which would otherwise feed back into the build as
  a doubled header.
- `.ptah/config.toml` semantics are unchanged: user-authored, created
  once, skipped with a message on re-run, never modified.
- Per-file output lines become `created:` / `updated: … (old -> new)` /
  `updated:` (no parseable old header) / `up to date:` for the
  definitions, alongside the existing `created:` / `skipped (exists):`
  for the config. One line per file, hints on every run, exit codes
  unchanged (0 success, 1 write failure).
- `INIT_HINTS` and the README teach re-running `ptah init` as the
  upgrade path; `ptah types > .ptah/ptah.d.luau` remains documented as
  the alternative (scripting, or refreshing without touching config).
- Glossary terms adopted in `CONTEXT.md`: source / embedded / project
  definitions; init scaffolds the registry skeleton but syncs the
  project definitions.

Not a **BREAKING** change in exit codes or file set, but the observable
behavior of `ptah init` against an existing `.ptah/ptah.d.luau` changes:
it now may overwrite where it previously skipped.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `cli`: the init requirement's existing-file semantics are split by
  ownership — config stays skip-only; the project definitions are synced
  (create / update / no-op-on-byte-identity, including the headerless
  source layout). The "running `ptah init` twice SHALL leave previously
  existing files byte-identical" clause is reworded to same-binary
  idempotence. New scenarios: stale definitions updated with a version
  arrow; edited/headerless non-matching file overwritten; source-layout
  file left untouched.
- `type-definitions`: the editor-setup documentation requirement now
  documents re-running `ptah init` as the primary refresh path after
  upgrading, with `ptah types > .ptah/ptah.d.luau` kept as the
  documented alternative (not demoted to a footnote).

## Impact

- `crates/ptah-cli/src/cli.rs` — `run_init` (sync logic, version parsing
  from the old header, message set), `INIT_HINTS` text; unit tests for
  the message/format helpers.
- `crates/ptah-cli/tests/init.rs` — `rerunning_init_reports_skips_and_is_idempotent`
  splits into config-skip + definitions-sync cases; new tests for the
  stale-update, edited-overwrite, and source-layout no-op scenarios.
- `README.md` — the `ptah init` / editor-setup paragraphs.
- No new dependencies, no CLI-surface changes (no new flags), no changes
  to `ptah types`, `ptah check`, or the registry.
- Out of scope by decision: staleness detection/warnings in `run`/`check`.
