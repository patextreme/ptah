# Tasks

## 1. Dependencies

- [x] 1.1 Add `petname = { version = "3.2", default-features = false, features = ["default-rng", "default-words"] }` to the root `Cargo.toml` workspace table with a justification comment, and to `crates/ptah-render/Cargo.toml`; add `rand = "0.10"` to the workspace table and to `crates/ptah-render/Cargo.toml` (petname does not re-export `rand`); remove the `getrandom` entry from both. Verify: `nix develop -c cargo metadata` resolves and the lock adds only `petname` + `petname-macros` (no rand version fork).

## 2. Run id minting

- [x] 2.1 Rewrite `RunId::mint` in `crates/ptah-render/src/record.rs`: UTC prefix `%Y%m%d-%H%M%S`, suffix from `Petnames::small().namer(2, "-").iter(&mut rand::rng()).next().expect("small word lists are non-empty")`, unchanged occupied-directory re-mint loop; drop the `io::Result` if no fallible call remains and update the `RunId::mint` call site at `crates/ptah-render/src/record.rs` (`RunRecord::create`; `crates/ptah-cli/src/cli.rs` calls `RunRecord::create` and needs no change). Verify: `cargo build`.
- [x] 2.2 Update the four §2.2 unit tests (`run_id_shape_*` becomes the word-pair shape test, UTC test pins `20260912-143022-`, sort-order and occupied-id tests carry over, dropping the now-unnecessary `.unwrap()` on the infallible `RunId::mint` calls), the module doc comment that describes the id format, and the stale old-format id literals in the `fully_populated` and `run_start_line` fixtures. Verify: `nix develop -c cargo test -p ptah-render record`.
- [x] 2.3 Update the run-id shape assertion in `crates/ptah-cli/tests/record.rs` (`run_creates_a_log_and_run_json`, around lines 121-126), which currently asserts a 14-digit timestamp prefix and an all-digit suffix. The new id is `yyyymmdd-hhmmss-<adjective>-<noun>`: split off the `yyyymmdd-hhmmss` prefix and assert the suffix is two lowercase word tokens. Verify: `nix develop -c cargo test -p ptah-cli --test record`.

## 3. Docs and spec sync

- [x] 3.1 No manual edit to `openspec/specs/run-record/spec.md`: `openspec archive` syncs this change's delta into it. After archiving, confirm the synced "Run ids are sortable and collision-free" requirement and its scenarios read the new `yyyymmdd-hhmmss` + adjective-noun shape. *(As-built: the main spec is untouched as required; `openspec validate word-suffix-run-ids --strict` passes, so the delta is ready to sync. The post-archive confirmation is the archive phase's step, not this apply pass.)*
- [x] 3.2 Update `README.md` — the run-record section prose (id format), the sample run-start line (`[ptah] run record: .ptah/runs/20260912-143022-polite-aardvark`), and any other `yyyymmddhhmmss` mention. Verify: `rg -n "yyyymmddhhmmss|20260912143224" README.md` returns nothing.

## 4. Full verification

- [x] 4.1 Run the full suite offline: `nix develop -c cargo test` (the run-id shape assertion in `crates/ptah-cli/tests/record.rs` is updated in 2.3; all other record tests derive the id dynamically). Verify: green. *(As-built: `cargo test --no-fail-fast` is green for every target this change touches and all others; the sole failure is the pre-existing, unrelated `flake_pin_matches_the_committed_lockfile_tree` guard — HEAD's `e6233eb` refreshed `.ptah/pesde.lock` to `e21ec3d` without updating the `ptah-libs` pin in `flake.nix`, and no file this change touches is involved.)*
- [x] 4.2 Run `nix develop -c cargo clippy` and `nix develop -c nix build` (or `nix flake check`) to confirm the new dependency builds in the sandbox. Verify: green.
