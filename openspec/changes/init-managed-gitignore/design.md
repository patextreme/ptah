## Context

`ptah init` (`crates/ptah-cli/src/cli.rs`, `run_init`) currently writes three files with two ownership modes: user-authored configs (`created:`/`skipped (exists):`, never modified) and a derived artifact (`ptah.d.luau`, synced: `created:`/`updated:`/`up to date:`). The run-record layer already writes one ignore file — the `runs/` enclave (`.ptah/runs/.gitignore` = `*`, create-if-absent, never modified; `ptah-render/src/record.rs`). The package-management layer prints commit/ignore guidance on the first mutating command and writes nothing. See proposal.md for motivation.

Two glossary terms govern this design (`CONTEXT.md`): **enclave ignore** and **managed ignore section**.

## Goals / Non-Goals

**Goals:**

- The generated package paths are ignored from the moment the manifest that guarantees them is scaffolded, in every project and every clone.
- `.ptah/.gitignore` remains a file the user may own content in; ptah's writes are confined to its marked section.
- One writer: `ptah init` is the only command that writes ignore content; package commands stay print-only.
- The ignore rule travels with the repository (committed file), unlike a per-clone-local rule inside ignored content.

**Non-Goals:**

- Per-directory `*` enclaves inside `luau_packages/`/`.pesde/` (rejected: the rule would live in engine-owned cache, never committed, and would not protect a fresh clone before its first install).
- Install-time repair writes (rejected: package commands keep the print-only posture; the legacy repair path is re-running init, which the upgrade hints already prescribe for definitions).
- Opt-out machinery — flag, manifest key, or sentinel line (rejected: hypothetical vendoring audience, real contract surface; the documented behavior is that the section returns on init re-run).
- Covering `runs/` (it keeps its enclave; one ignore mechanism per directory).
- Writing or probing the repository root ignore file, or detecting git (worktrees/submodules make detection unreliable; an inert file in a non-git tree is noise, matching the runs/ precedent of unconditional writes).

## Decisions

**D1 — Managed section, not a wholly-owned file.** `.ptah/.gitignore` carries a marker-delimited ptah block; content outside the markers is user-owned. Alternative rejected: whole-file ownership with create-if-absent — it would make every legacy or hand-rolled file a permanent skip, and it could never grow (the list is static forever). Markers compose init's two existing ownership modes into one file: user rules behave like `config.toml` (never touched), the section behaves like `ptah.d.luau` (derived, synced).

**D2 — Refresh semantics (create / append / rewrite-between-markers).** The section is derived content, so the named-list maintenance burden is paid automatically: a future generated path under `.ptah/` propagates on the next init re-run. Alternatives: append-only-once (heals legacy once, then the list is frozen — worst of both) and skip-if-exists (markers become inert decoration).

**D3 — Exact section content:**

```gitignore
# >>> ptah (managed section; `ptah init` refreshes it)
/luau_packages/
/.pesde/
# <<< ptah
```

Anchored patterns (leading `/`) so they name exactly the two paths in `.ptah/`; unanchored `.pesde/` would also match the linker's nested `luau_packages/.pesde/` container (harmless but inexact). ssh-style `>>>`/`<<<` markers for recognition; the opening marker carries the maintenance contract — the only place a file-level reader learns the refresh behavior. A file created by init contains exactly this section and nothing else. Marker recognition for the append/refresh logic: opening = a line beginning `# >>> ptah`, closing = the line `# <<< ptah`.

**D4 — Status-line vocabulary mirrors the derived-artifact mode.** `created:` (file absent), `appended:` (file exists, no markers), `updated:` (markers present, content differs), `up to date:` (markers present, content matches). The line names `.ptah/.gitignore` like every other per-file line. Idempotence holds: same binary, second run reports `up to date:` and writes nothing.

**D5 — Append mechanics.** When appending to an unmarked existing file, the section is appended with a preceding blank line if the file does not already end with one, and never rewrites existing bytes. When refreshing, everything strictly between the markers is replaced; if multiple marker pairs exist, the first opening marker and the first closing marker after it bound the section and later pairs are left alone (degenerate input, not worth failing on). Trailing-newline normalization applies only inside the rewritten region.

**D6 — Failure posture is init's existing per-file contract.** Any failure writing or rewriting `.ptah/.gitignore` (unwritable dir, `.gitignore` exists as a directory, marker line unreadable due to I/O error) prints an error to stderr and exits 1. Not warn-and-continue: the file is protection, and half the point is that the guarantee is uniform. This matches `runs/`'s strictness ("a record whose ignore file cannot be written gets no record at all") and init's behavior for its other three files.

**D7 — Guidance becomes confirmation.** `GUIDANCE` in `crates/ptah-cli/src/package.rs` is rewritten to name the commit list (`pesde.toml`, `pesde.lock`, root `.luaurc`, `.ptah/.gitignore`) and state that the managed section already ignores the generated dirs. Rationale: guidance prints only on the *first* mutating command, which post-change is always a project whose init already wrote the section — its instructional half is void; its remaining job is naming the commit rows, and `.ptah/.gitignore` must be among them because its protection is void if not committed. The unit test pinning GUIDANCE contents moves with it.

## Risks / Trade-offs

- [No durable opt-out: deleting the file or section is undone by the next init re-run] → Documented behavior (proposal, README); the affected audience is a project that deliberately commits `luau_packages/` as vendored source, which can delete the section after the rare init re-run. Revisit with real users, not machinery.
- [A user who edits inside the markers loses the edit on the next init] → The markers declare the contract; the opening-marker comment says so in place. This is the same trade `ptah.d.luau` already makes for hand-edits.
- [Committed-file dependence: an uncommitted `.ptah/.gitignore` protects nothing for clones] → Mitigated by the guidance naming it in the commit list (D7) and the README state table gaining a **commit** row.
- [Marker-format drift vs. hand-written near-miss markers (e.g. `# >>>ptah`)] → Recognition is exact-prefix on the opening line and exact-match on the closing line; a near miss reads as "no markers" and gets the section appended — safe (duplicate sections are inert ignore rules).

## Migration Plan

Legacy projects (scaffolded before this change, or hand-rolled rules) heal by re-running `ptah init` — the same command the upgrade hints already prescribe for stale definitions; a hand-rolled unmarked `.ptah/.gitignore` gains the section without losing a byte. No lockfile or manifest format changes; rollback is deleting the section (file) — with the documented caveat that a future init re-run restores it.
