## 1. Adapter crate skeleton

- [x] 1.1 Create `crates/ptah-pesde` (lib, workspace member) with `pesde =0.7.4` (`default-features = false`), `toml_edit`, `tokio`, `reqwest`, `serde_json`, `thiserror`-style error type mapping pesde/IO/manifest failures; verify `cargo build` passes with no libgit2/wally features in the tree (`cargo tree -i git2` reports nothing)
- [x] 1.2 Implement project discovery (walk up from invocation dir for `.ptah/` containing `config.toml` or `pesde.toml`) and `Project` construction with `.ptah/.pesde/data` + `.ptah/.pesde/cas`; verify with unit tests over a tempdir fixture tree (found at root, found from nested, not found → error)
- [x] 1.3 Implement manifest skeleton (`components/<sanitized-dirname>`, `private = true`, `environment = "luau"`, no `[indices]`) and skeleton-parse test against pesde's `Manifest` deserialization

## 2. Pesde driver

- [x] 2.1 Implement the default-index injection (deserialize manifest; when `indices` empty, inject the compiled-in default URL in-memory) and toml_edit-based manifest read/modify/write preserving unknown tables; verify a unit test round-trips a manifest with a user `[indices]` table byte-identically
- [x] 2.2 Implement the install driver (reuse-lockfile `dependency_graph`, `download_and_link` with no-op reporters, build + write `Lockfile`); verify against a local `--path`-source package in a tempdir (no network)
- [x] 2.3 Implement the locked-mode staleness check (missing lockfile; manifest name/target/overrides/direct-specs vs lockfile); verify unit tests covering missing, stale, and fresh lockfiles
- [x] 2.4 Implement `update` (fresh `dependency_graph(None, ..)`) and `add` version selection (newest via `PackageSource::resolve`, record `^<version>`); verify with `--path`/git fixtures that the recorded spec and lockfile pin are correct

## 3. Alias plumbing (`@alias` requires)

- [x] 3.1 `ptah-luau`: enable `.luaurc` discovery in `ScriptRequier` (`has_config`/`config`) and implement alias landing in `jump_to_alias`/path resolution; verify runtime tests for sibling/out-of-tree (regression) + alias-resolves, alias-discovered-upward, unknown-alias-rejected (spec: scripting "Relative module resolution")
- [x] 3.2 `ptah-check`: duplicate the alias walk-up purely in the lint walker (nearest `.luaurc`, `aliases` table, resolve relative to config dir); verify lint tests for alias-over-installed-package (no finding) and undefined alias (finding) (spec: script-checking "Static lints")
- [x] 3.3 Verification spike + fix: confirm `luau-lsp analyze` resolves `@alias` requires through the project-root `.luaurc`; if not, pass `--base-luaurc`; verify with a strict script whose aliased package member misuse produces a luau-lsp diagnostic (spec: script-checking "Typecheck pass")
- [x] 3.4 Implement `.luaurc` sync in `ptah-pesde` (create when absent; replace only ptah-owned entries; preserve user keys/aliases; collision → diagnostic, user wins); verify a unit-test matrix: absent, user-keys-preserved, user-alias-collision, remove-deletes-entry

## 4. CLI surface

- [x] 4.1 Add the `ptah package` group (`add`, `remove`, `install`, `update`; flags `--as`, `--git`, `--rev`, `--path`, `--no-install`, `install --locked`) wiring to the adapter with ptah-style diagnostics and the 0/1/2 exit-code contract; verify usage-error tests (missing arg, unknown remove alias, no project → 2) and the tokio runtime composition
- [x] 4.2 Amend `ptah init`: write `.ptah/pesde.toml` skeleton when absent (skip-with-message when present), stay offline, update `INIT_HINTS` with the package workflow; verify the init test suite extended per the cli delta (fresh init creates three files, manifest skeleton valid, partial scaffold completes, idempotent re-run, `ptah init` with no network)
- [x] 4.3 Extend the completions visible-set test to include `package`; verify `cargo test --test cli` (or the pinning test) passes
- [x] 4.4 Add the commit/ignore guidance line to the first successful mutating package command (names `pesde.toml`, `pesde.lock`, `.luaurc`, `luau_packages/`, `.pesde/`); verify an e2e assertion that the guidance prints once

## 5. Offline e2e fixtures

- [x] 5.1 Build the fixture-registry test helper: a generated local git index + in-process loopback archive server serving fixture packages, plus a local bare git repo for `--git` sources; verify the helper installs a fixture package into a tempdir project with no external network (offline)
- [x] 5.2 Write the e2e: clean project → `ptah init` → `ptah package add <fixture>` → require `@<alias>` from a workflow → `ptah check` clean → `ptah run` against mock-agent exercises the package; verify the full scenario passes inside `cargo test` (spec: package-management scenarios)
- [x] 5.3 Write the locked e2e: fresh clone (delete `luau_packages/` + `.pesde/`) → `ptah package install --locked` restores byte-identical packages, lockfile untouched; stale manifest → exit 1 with diagnostic

## 6. Docs, guards, nix

- [x] 6.1 README: package-management section (commands, layout, commit/ignore guidance, git/path source forms incl. the ptah-libs pattern), amend the factory-components "no registry, no lockfile" distribution paragraph to name both channels; verify doc links and the guidance match the implemented output
- [x] 6.2 `skills/ptah/SKILL.md`: add the package workflow (init → add → require via `@alias` or relative → check); verify the skill's examples typecheck against `.ptah/ptah.d.luau` where applicable
- [x] 6.3 AGENTS.md: clarify the offline invariant wording (loopback test doubles are the established pattern) and add the `ptah-pesde` crate to the architecture list; verify `nix flake check` source whitelisting covers the new files (`nix/source.nix`) and the Luau gates still pass
- [x] 6.4 Run the full gates: `cargo test` (all suites), `stylua --check .`, `ptah check` over touched `.luau` files, `nix flake check`; verify everything green
