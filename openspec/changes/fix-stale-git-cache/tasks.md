## 1. Cache normalization helper

- [ ] 1.1 Add a private helper in `crates/ptah-pesde/src/driver.rs` that enumerates `<data_dir>/git_repos/*`, opens each bare repo with `gix`, and appends `+refs/heads/*:refs/heads/*` to the default remote's fetch refspecs when that mapping is absent (idempotent; skips unreadable/non-repo entries; short-circuits when the directory is missing). Verify: a unit test builds a bare repo via the fixture, asserts the refspec is added on first call and that a second call leaves the config unchanged.
- [ ] 1.2 Preserve existing refspec order (the original `+refs/heads/*:refs/remotes/origin/*` must remain first, for `root_tree`). Verify: a unit test asserts `remote.refspecs(Fetch).first()` is still the remote-tracking refspec after normalization.

## 2. Wire into the driver

- [ ] 2.1 Call the helper at the top of `install()`. Verify: existing `install`/`update`/`remove` tests in `driver.rs` still pass (`cargo test -p ptah-pesde`).
- [ ] 2.2 Call the helper at the top of `add()` before its refresh/resolve pass. Verify: existing `add` tests in `driver.rs` still pass.

## 3. Regression tests (offline, evolving fixture)

- [ ] 3.1 Add a test: add an unpinned git dependency, advance the fixture branch to new contents, run `update`, and assert the lockfile's pinned tree and the installed files match the new commit. Verify: the test fails against the pre-fix helper (stale tree) and passes after.
- [ ] 3.2 Add a test: create a cache for a git repo that has no manifest, advance the branch to add the manifest, then run `add --rev <branch>` and assert it resolves and installs at the new tip. Verify: the test reproduces the `no manifest found` failure before the fix and passes after.

## 4. Verification

- [ ] 4.1 Run the crate and integration suites in the dev shell: `nix develop -c cargo test -p ptah-pesde` and `nix develop -c cargo test --test e2e`. Verify: all pass.
- [ ] 4.2 Run `nix flake check` (formatter, clippy, and the offline suite in the sandbox). Verify: passes with no new warnings.
- [ ] 4.3 Confirm the delta spec's scenarios are each covered by a test or an existing test. Verify: map `openspec/changes/fix-stale-git-cache/specs/package-management/spec.md` scenarios to test names.
