# Design: `ptah init` syncs the project definitions

## Context

See `proposal.md` — Why. Today `run_init` in `crates/ptah-cli/src/cli.rs`
loops over two `(name, contents)` pairs and skips any file that exists;
the definitions and the config skeleton get identical skip treatment even
though only the config is user-authored. The emitted definitions are
built by `definitions_bytes()` = one header line (`-- ptah <VERSION> type
definitions`) + `defs::TYPE_DEFINITIONS` (the `include_str!` of the
repo's headerless `.ptah/ptah.d.luau`). That headerless source file is a
compile input, which is why naive overwriting is unsafe in this repo (see
D2).

## Goals / Non-Goals

**Goals:**

- `ptah init` becomes the upgrade path for the project definitions:
  re-running it after upgrading leaves the editor type-checking against
  the installed binary's API.
- Preserve the invariants that already hold: exactly two files, one line
  per file, hints every run, exit 0/1 semantics, config never modified.
- Protect the repository's own source definitions from being rewritten
  by `init` run at the repo root.

**Non-Goals:**

- Staleness detection or warnings in `run`/`check` (out of scope by
  decision; the version header remains greppable).
- Any change to `ptah types`, `ptah check`, the registry, or the
  definitions content itself.
- Flags (`--force`, `--no-update`, `--dry-run`) — rejected; `ptah types
  >` already serves as the manual escape hatch.

## Decisions

**D1 — Sync predicate is byte-equality, not provenance.** The existing
file is left untouched only when it equals the current emitted output,
or equals the emitted output minus its header line; anything else is
overwritten. Alternatives rejected: (a) a header gate — overwrite only
when the first line is a ptah version header — preserves hand edits but
was rejected as the wrong default for a file documented as generated;
(b) "overwrite only when the old content matches some previously emitted
version" is unverifiable without shipping version history.

**D2 — The headerless arm exists to break a compile feedback loop.** The
repo's `.ptah/ptah.d.luau` has no version header (the header is prepended
at emit time). Under unconditional overwrite, `ptah init` at the repo
root would rewrite it *with* a header; the next build would then embed
`header + (header + body)` and `ptah types` would emit a doubled header —
permanently. Matching "emitted output sans header line" makes the source
file report `up to date` and never be written. This is not a provenance
check (see D1): a consumer file that is stale, hand-edited, or foreign
cannot match either accepted form and is still overwritten.

**D3 — Version suffix parsed from the overwritten file's first line.**
When the old file's first line matches the emitted header shape (`-- ptah
<version> type definitions`), the `updated:` line carries `(old ->
current)`; otherwise plain `updated:`. ASCII `->`, matching the CLI's
plain-text output elsewhere. Parsing is strict: an exact prefix/suffix
match with a non-empty middle, anything else counts as unparseable — a
malformed header never blocks the overwrite, only the suffix.

**D4 — Messages and exit codes stay a closed set.** Per file, exactly one
line: config `created:` / `skipped (exists):`; definitions `created:` /
`updated:` / `up to date:`. Hints after, every run. Exit 0 for all
non-I/O outcomes (including "nothing changed"), 1 on write failure —
unchanged from today. No new states, so scripting on init output keeps
working.

**D5 — Plain `fs::write`, no temp-file rename.** Atomic rename would
guard against a crash mid-write, but the created-path already uses a
plain write today, the payload is a few KB, and a truncated file is
self-healing (re-run init, or `ptah types >`). Consistency with the
existing path wins; revisit only if init ever writes something larger.

**D6 — Implementation shape: special-case the definitions inside the
existing loop.** `run_init` keeps its two-entry loop for the config;
the definitions get a `sync_definitions()` helper (read-if-exists →
compare against both accepted forms → write-or-not → return the message
line) plus a small `parse_defs_header()` for D3. Both are unit-testable
in the `cli.rs` tests module without touching the filesystem-backed e2e
suite.

## Risks / Trade-offs

- [Hand-edited project definitions are clobbered] → Accepted by
  decision: the file is documented as byte-identical generated output;
  `ptah types > .ptah/ptah.d.luau` remains the documented way to pin a
  custom variant, and re-running init restores the canonical form.
- [Header parse disagrees with what some old binary emitted] → Worst
  case is a missing version suffix, never a skipped sync; the header
  format has been stable since the definitions subcommand existed.
- [Tests fabricate "stale" files instead of using an old binary] →
  Acceptable: the sync predicate only sees bytes; a hand-written old
  header + differing body is indistinguishable from a genuine old emit.
- [Repo-root `init` writes despite D2] → Pinned by a dedicated test
  (source-layout file reported `up to date`, bytes unchanged).

## Migration Plan

None: no stored state, no wire or config format changes. Rollback =
revert the change; every previously written file remains valid. A
consumer who wants a custom definitions variant keeps it out of init's
reach by refreshing with `ptah types >` after any init run (or simply
not re-running init).
