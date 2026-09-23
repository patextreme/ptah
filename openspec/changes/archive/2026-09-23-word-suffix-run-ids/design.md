# Design

## Context

`RunId::mint` in `crates/ptah-render/src/record.rs` owns the id: a UTC
`%Y%m%d%H%M%S` prefix plus a `getrandom`-drawn 4-digit decimal suffix, re-minted
in a loop while the directory exists. The id is consumed as an opaque string by
`run.json` and the run-start line; nothing in ptah parses its shape. The
workspace already carries `rand 0.10` in the lock (via quinn-proto under
reqwest) and `getrandom` as a direct ptah-render dependency whose only caller
is the decimal suffix this change deletes. See proposal.md — Why.

## Goals / Non-Goals

**Goals:**

- A human-grippable id (`20260912-143022-polite-aardvark`) that keeps every
  existing guarantee: lexicographic sort = start order, UTC encoding,
  occupied-id re-mint, same-second relative order unspecified.
- Word quality for free: adjective-noun pairs whose entire cross-product is
  safe to print.
- One RNG in the tree, not two generations.

**Non-Goals:**

- Migrating or renaming existing record directories.
- Making ids meaningful (seeded from script name, project, or anything else).
- Changing `run.json`'s schema or anything else about the record's layout.
- Local-time timestamps (stays UTC; a deliberate, tested spec property).

## Decisions

- **Word source: the `petname` crate (3.2), `Petnames::small()`.**
  Alternatives: (a) `names` crate — dormant since 2022, hard-depends on
  `rand 0.8` (a second rand generation in the lock) and hides `clap 3` in its
  default features; rejected. (b) Vendor petname's word lists as const arrays
  — zero packages, but ~900 unreviewable lines plus attribution glue; the
  curation trust is delegated either way and the frozen copy forfeits upstream
  updates; rejected. (c) Hand-curated lists — we would own the adjective×noun
  cross-product safety review that petname 2.0 famously had to redo; rejected
  unless house tone ever matters more than the review burden.
  `namer(2, "-")` yields exactly adjective-noun (verified in petname 3.2.0's
  source: `words = 2` maps to `Adjective` then `Noun`; generation goes through
  `Namer::iter(rng)` or `Namer::generate_into(buf, rng)`). The `small()` list is
  449×449 ≈ 201k combinations — 20× today's collision space; worst-case id is
  33 chars (`20260912-143022-absolute-aardvark`), all lowercase ASCII.

- **Dependency shape: `petname = { version = "3.2", default-features =
  false, features = ["default-rng", "default-words"] }` plus a direct
  `rand = "0.10"` in the workspace table.**
  Default features off drops petname's own CLI (clap); `default-words` keeps
  the word-embedding macro, `default-rng` enables rand's `thread_rng` feature.
  `rand` must also be a direct dependency of `ptah-render`: petname does not
  re-export it, the mint constructs the RNG itself, and lock presence alone
  does not put a crate in the extern prelude. Net-new packages: `petname` +
  `petname-macros` only — `rand 0.10` is already resolved in the lock via
  quinn-proto, so no RNG version fork. Declared in the workspace manifest with
  a justification comment, house-style.

- **`getrandom` leaves the workspace dependency table.** Its sole caller was
  the decimal suffix; with thread_rng the mint has no fallible randomness
  source, so `RunId::mint` also loses its `io::Result` (the record creation
  path keeps its own error handling). The stale "decimal collision breaker"
  comment would be worse than no declaration. `getrandom` remains in the lock
  transitively via `rand`.

- **`SCHEMA_VERSION` stays `1`.** The id is an opaque string in `run.json`;
  the object's shape is unchanged. The id-format change is versioned by this
  spec delta, not by the JSON schema.

- **Timestamp: UTC, now with a hyphen (`yyyymmdd-hhmmss`).** The zone is an
  existing, deliberately tested property — one behavior changes at a time.
  The added hyphen is the legibility win the format asks for; fixed-width
  fields keep the lexicographic-start-order guarantee intact.

- **No migration.** Old ids and new ids coexist; new-format ids sort before
  same-date old-format ids (`-` = 0x2D sorts below digits). Write-once
  artifacts in a self-ignoring directory: the blip self-heals.

## Risks / Trade-offs

- [Two-word suffix widens the id] → Worst case 33 chars vs 19 today; terminal
  lines and `ls` stay well within comfort. No filesystem concern.
- [Word-pair collisions per second] → 201k space vs 10k today; the re-mint
  loop is unchanged and remains the correctness backstop.
- [petname pulls a proc-macro (`petname-macros`) into the build graph] →
  Tiny, same-publisher, compiled once per build; accepted in exchange for
  maintained, cross-product-safe lists.
- [External consumers assuming the old id shape] → Out of ptah's control and
  unobserved (nothing in-repo parses it); called out as **BREAKING** in the
  proposal.

## Migration Plan

Single commit behind the ordinary release flow; no data migration. Rollback is
a revert — ids are write-once and never parsed, so mixed directories from
before/after are already the designed steady state.

## Open Questions

(none)
