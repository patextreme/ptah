## Why

Unpinned git dependencies (`ptah package add --git <url>` with no `--rev`, or `--rev <branch>`) are resolved against a **stale local branch ref** in ptah's bare-clone cache. `ptah package update` therefore never advances them to newer commits, and `--rev <branch>` can fail with a misleading `no manifest found in repository` error when the cached branch predates a manifest change. This contradicts ptah's documented intent (unpinned git deps track the branch tip; only installs stay pinned) and was observed in real use against `patextreme/ptah-libs`.

The cause is an unfixed defect in the embedded Pesde 0.7.4 engine, which is the latest release and whose `main` still has the same logic — so ptah must compensate in its own adapter.

## What Changes

- Repair ptah's git dependency cache so that fetching advances the **local branch refs**, not only `refs/remotes/origin/*`, before every package operation that can resolve a git source (`install`, and `add`'s own resolve pass; `update`/`remove` funnel through `install`).
- As a result: `ptah package update` advances an unpinned (`rev = "HEAD"`) git dependency to the branch tip; `ptah package add --git <url> --rev <branch>` succeeds against a cache whose local branch is stale; `ptah package install` continues to install exactly the lockfile pin (installs stay pinned; only `update` moves).
- Offline regression tests covering both symptoms using the existing evolving-git-repo fixture.
- No new command, flag, or user-facing surface. No manifest/lockfile format change.

Non-goals: forking or patching Pesde; changing Pesde's own `NoManifest` error message (not reachable without a fork — the fix removes the stale-cache cause instead); changing how pinned commit SHAs resolve.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `package-management`: the `package add`, `package install`, and `package update` requirements gain the git-dependency freshness contract — branch-tracking resolution against the freshly fetched tip, and the install-pinned / update-advances split.

## Impact

- `crates/ptah-pesde/src/driver.rs` — new cache-normalization helper plus call sites in `install()` and `add()`; a small coupling to Pesde's cache layout (`data_dir/git_repos/<hash>` and its remote fetch refspec), documented in `design.md`.
- `crates/ptah-pesde/src/fixtures.rs` — no change expected (the evolving-branch fixture already supports building the regression tests); tests land beside the existing git-source tests in `driver.rs`.
- No dependency changes (uses the already-present `gix` 0.73 and public `pesde` types). `ptah-core` and its port set are untouched.
