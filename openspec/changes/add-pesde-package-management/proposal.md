## Why

Factory Components are consumed today only by mounting source (flake input + symlink, submodule, vendored copy), so dependency acquisition, updates, and version pinning are left to every consuming repo. Issue #15 asks for first-class package management: `ptah` should install, version, and update components itself, without requiring a separately installed package-manager binary. This reverses the archived `2026-09-04-factory-components` "no registry, no lockfile" stance by adding a managed distribution channel — source mounting stays supported.

## What Changes

- New `ptah package` command group — `add`, `remove`, `install`, `update` — embedding [Pesde](https://pesde.dev) as a Rust library (no external `pesde` binary required).
- Package project layout inside `.ptah/`: `pesde.toml` manifest (user-authored, committed), `pesde.lock` lockfile (committed), `luau_packages/` (generated, ignored), `.pesde/` cache (generated, ignored).
- `ptah init` additionally scaffolds `.ptah/pesde.toml` when absent (never overwrites; stays offline). The "exactly two files" init contract changes to three.
- Installed packages become requirable from workflows two ways: pesde-native relative requires (already work today) and new `@alias` requires resolved through a ptah-synced `.luaurc` at the project root with standard Luau walk-up semantics — one file read identically by the runtime, `ptah check`'s lint walker, luau-lsp, and editors.
- New adapter crate `crates/ptah-pesde` pins `pesde =0.7.4` with `default-features = false` (no libgit2, no wally); git sources (e.g. `github.com/patextreme/ptah-libs`) and registry tarballs remain fully supported through pure-Rust gix/reqwest.
- The default Pesde registry index is supplied by ptah internally; the manifest carries no `[indices]` table unless the user adds one.

Non-goals (this change): publishing official components to a registry (git sources serve until then), dev-only dependencies, per-package `update`, `list`/`outdated` commands, and Roblox/Wally target support (the manifest target is pinned to `luau`).

## Capabilities

### New Capabilities

- `package-management`: the `ptah package add|remove|install|update` command group — project layout, manifest/lockfile ownership, locked installs, source forms (registry/git/path), alias defaults and `.luaurc` sync, exit codes, and source-control guidance.

### Modified Capabilities

- `cli`: `ptah init` scaffolds `.ptah/pesde.toml` as a third file (create-when-absent semantics like `config.toml`); the visible subcommand set gains `package`.
- `scripting`: require resolution accepts `@alias` requires via `.luaurc` discovery (standard Luau walk-up), in addition to relative paths; non-relative, non-alias requires remain rejected.
- `script-checking`: the static require-graph walk resolves `@alias` requires through the same `.luaurc` rules so installed packages check cleanly end-to-end.

## Impact

- **New crate** `crates/ptah-pesde` (adapter, `ptah-config` pattern): pesde driver (resolve/install/link, locked-mode staleness check, lockfile write), manifest edits via `toml_edit`, `.luaurc` sync, ptah-style diagnostics. No new port in `ptah-core` — the port set stays closed; the crate is composed from `ptah-cli`.
- **Dependencies**: `pesde =0.7.4` (pinned exact; no documented library-stability promise — 0.8 will break `Project::new`; the adapter isolates it), `toml_edit`, tokio (already in the run path).
- **`ptah-luau`**: `ScriptRequier` enables `.luaurc` config discovery and alias jumps.
- **`ptah-check`**: lint walker gains pure alias resolution (duplicating the walk-up on purpose, per house pattern).
- **`ptah-cli`**: new subcommand group; init changes; completions test set gains `package`.
- **Tests**: fully-offline suite gains loopback fixtures (local git index + in-process archive server), the `mock-agent` precedent; e2e installs a fixture package into a clean project and runs/checks a workflow that requires it.
- **Nix**: `nix/source.nix` whitelists the new files/dirs the sandbox build needs; flake check Luau gates keep passing.
- **Docs**: README distribution section amended (packages alongside mounting); `skills/ptah/SKILL.md` gains package workflow; `CONTEXT.md` already updated (`Package`, `Package alias`, generalized `Mount point`).
- **Git hygiene**: `.gitignore`/hints — commit `pesde.toml` + `pesde.lock` and the root `.luaurc`; ignore `luau_packages/` and `.pesde/`. ptah prints hints, never edits VCS config.
