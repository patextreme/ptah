# Tasks

## 1. Dependencies

- [ ] 1.1 Add `petname = { version = "3.2", default-features = false, features = ["default-rng", "default-words"] }` to the root `Cargo.toml` workspace table with a justification comment, and to `crates/ptah-render/Cargo.toml`; remove the `getrandom` entry from both. Verify: `nix develop -c cargo metadata` resolves and the lock adds only `petname` + `petname-macros` (no rand version fork).

## 2. Run id minting

- [ ] 2.1 Rewrite `RunId::mint` in `crates/ptah-render/src/record.rs`: UTC prefix `%Y%m%d-%H%M%S`, suffix from `Petnames::small().generate(&mut rand::rng(), 2, "-")`, unchanged occupied-directory re-mint loop; drop the `io::Result` if no fallible call remains and update the call site in `crates/ptah-cli/src/cli.rs`. Verify: `cargo build`.
- [ ] 2.2 Update the four §2.2 unit tests (`run_id_shape_*` becomes the word-pair shape test, UTC test pins `20260912-143022-`, sort-order and occupied-id tests carry over) and update the module doc comment that describes the id format. Verify: `nix develop -c cargo test -p ptah-render record`.

## 3. Docs and spec sync

- [ ] 3.1 Update `openspec/specs/run-record/spec.md` — rewrite the "Run ids are sortable and collision-free" requirement prose and its "Id shape" / "UTC regardless of the machine's zone" scenarios for the new prefix (done at archive time from this change's delta).
- [ ] 3.2 Update `README.md` — the run-record section prose (id format), the sample run-start line (`[ptah] run record: .ptah/runs/20260912-143022-polite-cymbol`), and any other `yyyymmddhhmmss` mention. Verify: `rg -n "yyyymmddhhmmss|20260912143224" README.md` returns nothing.

## 4. Full verification

- [ ] 4.1 Run the full suite offline: `nix develop -c cargo test` (integration tests discover ids dynamically and need no change). Verify: green.
- [ ] 4.2 Run `nix develop -c cargo clippy` and `nix develop -c nix build` (or `nix flake check`) to confirm the new dependency builds in the sandbox. Verify: green.
