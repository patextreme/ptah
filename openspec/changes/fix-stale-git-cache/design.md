## Context

See `proposal.md` — Why. Two constraints shape the approach:

- **The defect is in the pinned engine, not ptah.** Pesde 0.7.4 is the latest release and its `main` still refreshes git caches with the clone's default refspec (`+refs/heads/*:refs/remotes/origin/*`), so `refs/heads/<branch>` is frozen after the initial clone while `refs/remotes/origin/<branch>` advances. Pesde's `resolve` calls `rev_parse_single(rev)`, which for `HEAD`/`main` hits the frozen local branch; `root_tree` likewise keys off the first fetch refspec.
- **No seam between refresh and resolve.** `ptah-pesde` calls `dependency_graph`, which refreshes and resolves each source internally, per node. `add` does refresh and resolve itself, but `install`/`update` go through `dependency_graph` with no hook in between. So a fix cannot sit "between pesde's fetch and pesde's rev-parse" at the call site.

The one lever that makes pesde's *own* fetch advance local refs is the cached repository's fetch refspec — validated by hand: appending `+refs/heads/*:refs/heads/*` to a repro cache made `ptah package update` advance the tree and `ptah package add --rev main` succeed.

## Goals / Non-Goals

**Goals:**
- Make every git resolution observe the branch tip fetched in the same invocation, for `add`, `install`, and `update`, without forking or patching Pesde.
- Keep the workaround contained to `ptah-pesde`, invisible in the manifest/lockfile, and safe to run repeatedly.

**Non-Goals:**
- Changing Pesde's `NoManifest` error text (its error type is not reachable without a fork; the fix removes the stale-cache cause).
- Fixing Pesde upstream, or adding a `[patch.crates-io]` fork.
- Any new port, command, flag, or dependency.

## Decisions

### D1 — Repair the cached repo's fetch refspec, don't fork or pre-fetch
Append `+refs/heads/*:refs/heads/*` to the cached bare repo's default-remote fetch refspecs. Pesde's existing fetch then updates local branch refs, so its unchanged `rev_parse_single`/`root_tree` read current commits.

Alternatives considered:
- **Fork/vendor Pesde via `[patch.crates-io]`** — most "correct" (fix at the source), rejected: maintaining a fork of a 0.x crate for a one-refspec defect, and AGENTS.md treats a Pesde upgrade as a deliberate one-crate change, not a standing fork.
- **Record `refs/remotes/origin/HEAD` instead of `HEAD`** — fixes only the unpinned case (not `--rev <branch>`), and diverges from Pesde's manifest format and interop with real pesde.
- **Pre-fetch each cached repo with an explicit refspec each run** — no config mutation, but doubles the network fetch for every git source on every invocation.
- **Delete/repoint local refs** — `HEAD` is a symref to `refs/heads/<branch>`; removing it breaks `HEAD` resolution.

### D2 — Append, don't replace, and only when absent
Append to the refspec list so the original `+refs/heads/*:refs/remotes/origin/*` stays first: `root_tree` (used by registry-index sources) reads `refspecs(...).first()`, and preserving order keeps its behavior byte-identical. Check existing values first and write only when the local-heads refspec is missing, so repeat runs are no-ops and there is no config churn or lock contention.

### D3 — A driver pre-pass in `install()` and `add()`, not in `PackageProject::open`
Enumerate `<data_dir>/git_repos/*`, `gix::open` each bare repo, and normalize its default remote. Call it at the top of `install()` (covers `update` and `remove`, which funnel through install) and at the top of `add()` (its own resolve pass precedes its install). Doing it in `PackageProject::open` would add filesystem mutation to a pure constructor and run for commands that never touch git; a new port would be a core design decision for an adapter-local workaround.

The pass is best-effort and idempotent: skip unreadable/non-repo entries; a repo that does not exist yet is a no-op (Pesde's clone creates the local branch at the tip, so the first resolution is already correct, and the next invocation normalizes it).

### D4 — Regression tests on the evolving fixture
`fixtures::git_repo_with` already evolves a branch (parent tip carried forward), so both symptoms are reachable offline: (a) add unpinned → advance branch → `update` advances the lock tree; (b) cache a repo before its manifest exists → advance with manifest → `add --rev <branch>` succeeds. These live beside the existing git-source tests in `driver.rs`; no fixture changes expected.

## Risks / Trade-offs

- [Coupling to Pesde's private cache layout — the `git_repos` directory name and the refspec's meaning] → The engine is pinned exactly (`=0.7.4`) and already treated as upgrade-fragile; the regression tests exercise the real path, so a layout change surfaces as a failing test rather than silent staleness. The coupling is documented at the helper.
- [Force-updating local branch refs could discard state] → The cache is a bare, generated clone with no worktree; `+`-forced updates are the intended semantics for a mirror cache.
- [Scan cost on every package command] → A handful of `gix::open` calls over a small directory; a missing `git_repos` dir short-circuits.
- [A repo cloned mid-operation is not normalized until the next run] → Harmless: the clone's local branch is created at the fetched tip, so that invocation resolves correctly.

## Migration Plan

None required — the repair is transparent and self-healing on the next package command; no manifest, lockfile, or cache format changes. Rollback = remove the helper and its two call sites; caches keep working (with the original defect).
