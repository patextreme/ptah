# Proposal

## Why

Run record directories under `.ptah/runs/` are named with a bare decimal
suffix (`20260912143224-4821`) — opaque and easy to confuse at a glance when
scanning or discussing runs. A Docker/zellij-style word pair gives each run a
human-grippable handle (`20260912-143022-polite-cymbol`) while preserving the
properties the id exists for: sortability and collision-freedom.

## What Changes

- Run ids change from `yyyymmddhhmmss-<4 decimal digits>` to
  `yyyymmdd-hhmmss-<adjective>-<noun>` (e.g. `20260912-143022-polite-cymbol`).
  **BREAKING** for any consumer that assumed the old id shape; the id remains
  an opaque directory name for everything inside ptah.
- The timestamp prefix gains a hyphen between date and time; it stays UTC and
  fixed-width, so lexicographic order still equals start order.
- The random suffix becomes an adjective-noun word pair drawn from the
  `petname` crate's small, curated-safe word lists (449×449 ≈ 201k combinations,
  vs 10k today).
- `RunId::mint` loses its fallible randomness source: the `getrandom` workspace
  dependency is removed (its only caller was the decimal suffix); randomness
  comes from `rand`'s thread rng via petname's `default-rng` feature.
- `run.json`'s `SCHEMA_VERSION` stays `1`: the id is an opaque string in the
  JSON and the object's shape is unchanged.
- Existing record directories are not migrated; old and new ids coexist.
  New-format ids sort before same-date old-format ids (`-` sorts below digits)
 — an accepted, self-healing cosmetic blip.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `run-record`: the "Run ids are sortable and collision-free" requirement is
  rewritten for the new id shape — hyphenated UTC timestamp prefix plus a
  lowercase word-pair suffix; the sort-order, same-second-prefix, UTC, and
  no-reuse guarantees carry over unchanged.

## Impact

- `crates/ptah-render/src/record.rs` — `RunId::mint` rewrite and the §2.2
  unit tests that pin the id shape.
- Root `Cargo.toml` and `crates/ptah-render/Cargo.toml` — add `petname`
  (`default-features = false`, features `default-rng` + `default-words`;
  petname's own CLI/clap surface stays out), remove `getrandom`. `rand 0.10`
  is already in the lock via quinn-proto, so petname adds only itself and its
  word-embedding proc-macro.
- `openspec/specs/run-record/spec.md` — id-shape requirement and scenarios.
- `README.md` — run-record section and the sample run-start line.
- `CONTEXT.md` — untouched: "sortable, collision-free identifier" is
  format-agnostic.

References: GitHub issue #33.
