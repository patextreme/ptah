## Why

`ptah init` scaffolds `.ptah/pesde.toml` — the manifest that guarantees `.ptah/luau_packages/` and `.ptah/.pesde/` will exist — yet leaves their ignore rules to prose printed later by `ptah package install`. The failure lands at the first `git add -A` after the first install: ~552K of vendored package source plus a ~296K object cache swept into a commit, in a directory the README tells users to track. `.ptah/runs/` already solves the identical problem with a self-ignoring `.gitignore`; the package state gets the same treatment at init time, the moment the manifest that guarantees those paths is written. (Issue #26.)

## What Changes

- `ptah init` writes a fourth file, `.ptah/.gitignore`, containing a **managed ignore section**: a marker-delimited block (`# >>> ptah …` / `# <<< ptah`) with anchored patterns for the two generated paths (`/luau_packages/`, `/.pesde/`).
- The section is a **derived artifact** with the same ownership split init already has for whole files: content outside the markers is user-owned and never touched; the section itself is ptah-owned and **refreshed** — created when the file is absent, appended when the file exists without markers, rewritten between the markers when it exists with them.
- Writes are unconditional — no git detection, no flag, no manifest key. `runs/` is deliberately absent from the section; it keeps its own enclave mechanism (one ignore mechanism per directory).
- `ptah package install`'s first-install guidance is rewritten as confirmation: `.ptah/.gitignore` joins the commit list, and the ignore half states that the managed section already covers the generated dirs.
- The ignore-file invariant, replacing "ptah never edits ignore files itself": ptah never writes any ignore file outside `.ptah/`; inside, it touches only ignore content it can identify as its own — the `runs/` enclave file and the marked section — and never modifies anything else in a user's ignore file.
- No opt-out machinery: the section returns on init re-run; deletion is not durable and that is documented honestly. Legacy projects repair by re-running init — the same healing command the upgrade hints already prescribe for stale definitions.

Not doing: per-directory `*` enclaves inside the generated dirs (the rule would live in engine-owned cache and never travel with the repo); install-time writes (package commands print, never write ignore files); any opt-out flag or manifest key.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `cli`: the `ptah init` requirement's file set grows from exactly three to exactly four, gaining `.ptah/.gitignore` with managed-section refresh semantics (created/appended/refreshed, one status line per file, idempotent re-runs, hard-fail write posture).
- `package-management`: the exit-codes-and-guidance requirement's ignore sentence is replaced by the new `.ptah/`-scoped invariant, and the first-install guidance becomes confirmation naming `.ptah/.gitignore` in the commit list.

## Impact

- `crates/ptah-cli/src/cli.rs` — `run_init`: fourth create/append/refresh block, status-line vocabulary grows (`appended:`, `updated:` for the section), write failure exits 1 like every other init file.
- `crates/ptah-cli/tests/init.rs` — fresh init, existing-unmarked-file append, marker-refresh, idempotence, outside-markers-untouched, non-git unconditional write.
- `crates/ptah-cli/src/package.rs` + `tests/packages.rs` — GUIDANCE constant and its pinned-contents unit test.
- `README.md` — package state table gains a **commit** row for `.ptah/.gitignore`; "ptah never edits ignore files itself" replaced by the scoped invariant; guidance snippet.
- `CONTEXT.md` — terms `enclave ignore` and `managed ignore section` (already captured).
- `openspec/specs/run-record/spec.md` — untouched; `runs/` keeps its enclave mechanism unchanged.
