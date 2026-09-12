## 1. Cache normalization helper

- [x] 1.1 Add a private helper in `crates/ptah-pesde/src/driver.rs` that enumerates `<data_dir>/git_repos/*`, opens each bare repo with `gix`, and appends `+refs/heads/*:refs/heads/*` to the default remote's fetch refspecs when that mapping is absent (idempotent; skips unreadable/non-repo entries; short-circuits when the directory is missing). Verify: a unit test builds a bare repo via the fixture, asserts the refspec is added on first call and that a second call leaves the config unchanged.
- [x] 1.2 Preserve the existing fetch mapping: normalization appends, never replaces, so the original `+refs/heads/*:refs/remotes/origin/*` remains present. Note that gix returns `remote.refspecs(Fetch)` **sorted** (its read path sorts and dedups), so config order is not observable through that API — the added local-heads mapping sorts first. This is safe because `root_tree` (Pesde's only first-refspec consumer) is reached solely by Git-based index sources under `<data_dir>/indices`, which this helper never touches. Verify: a unit test asserts both mappings are present after normalization (none dropped).

## 2. Wire into the driver

- [x] 2.1 Call the helper at the top of `install()`. Verify: existing `install`/`update`/`remove` tests in `driver.rs` still pass (`cargo test -p ptah-pesde`).
- [x] 2.2 Call the helper at the top of `add()` before its refresh/resolve pass. Verify: existing `add` tests in `driver.rs` still pass.

## 3. Regression tests (offline, evolving fixture)

- [x] 3.1 Add a test: add an unpinned git dependency, advance the fixture branch to new contents, run `update`, and assert the lockfile's pinned tree and the installed files match the new commit. Verify: the test fails against the pre-fix helper (stale tree) and passes after.
- [x] 3.2 Add a test: create a cache for a git repo that has no manifest, advance the branch to add the manifest, then run `add --rev <branch>` and assert it resolves and installs at the new tip. Verify: the test reproduces the `no manifest found` failure before the fix and passes after.
- [x] 3.3 Add a guard test: with a git branch dependency pinned at commit A, advance the branch to commit B, run `install`, and assert the lockfile still records A and the installed files match A (installs stay pinned).
- [x] 3.4 Add a guard test: record a git dependency against an explicit commit SHA, advance the branch, run `update`, and assert the lockfile still records the SHA's tree and the installed files match it (commit pins do not move).

## 4. Verification

- [x] 4.1 Run the crate and integration suites in the dev shell: `nix develop -c cargo test -p ptah-pesde` and `nix develop -c cargo test --test e2e`. Verify: all pass.
- [x] 4.2 Run `nix flake check` — the offline suite (`ptah-tests`), the release smoke check, and the Luau StyLua/analyze/`ptah check` gates. Verify: passes. (The flake defines no Rust clippy or `cargo fmt` gate; the workspace suite was run separately via `cargo test --workspace`.)
- [x] 4.3 Confirm the delta spec's scenarios are each covered by a test or an existing test. Mapping:
  - add / subdirectory → `driver::tests::add_from_a_git_subdirectory_records_the_path`, `package_registry::add_from_a_git_source_with_a_subdirectory_e2e`
  - add / newest version → `package_registry::add_resolves_the_newest_version_and_pins_it`
  - add / unknown package → `package_registry::unknown_package_is_an_operational_error_naming_the_spec`
  - add / branch rev against a warm cache → `driver::tests::add_branch_rev_resolves_against_a_warm_cache`
  - install / idempotent → `driver::tests::install_is_idempotent_when_up_to_date`, `packages::package_install_is_idempotent_and_guidance_prints_once`
  - install / registry failure → `package_registry::registry_failure_is_an_operational_error`
  - install / keeps a git branch dependency pinned → `driver::tests::install_keeps_a_git_branch_dependency_pinned`
  - update / moves a caret within range → `package_registry::update_moves_a_caret_within_range`
  - update / advances an unpinned git dependency → `driver::tests::update_advances_an_unpinned_git_dependency`
  - update / keeps a commit-pinned git dependency → `driver::tests::update_keeps_a_commit_pinned_git_dependency`
