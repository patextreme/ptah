# Design: Pesde package management

## Context

Factory Components are consumed today only as mounted source (flake input + symlink, submodule, vendored copy). Issue #15 asks `ptah` to own dependency management by embedding [Pesde](https://pesde.dev) as a Rust library. Pesde (crate `pesde`, 0.7.4, MIT) is a single lib+bin crate: the library exposes `Project`, manifest/lockfile types, resolution (`dependency_graph`), and install (`download_and_link`) with injectable `reqwest` client and no-op reporter/hook traits; the CLI-layer behaviors (init skeleton, `add`/`remove` manifest edits, the `--locked` staleness check, lockfile writing) live in pesde's `src/cli/**` and are ~200 lines over public types that this change reimplements. Pesde's install layout for a `luau`-target project is exactly `.ptah/luau_packages/<alias>.luau` (generated linker modules with type re-exports) + `luau_packages/.pesde/<container>/` (hard-linked contents), with `<target>_packages/` naming hardcoded.

This change reverses the archived `2026-09-04-factory-components` "no registry, no lockfile" stance by adding a managed channel; source mounting remains supported (each package tree still only requires within itself, so mount-point freedom is preserved).

A research report on pesde 0.7.4's API (verified against source) sits at `.work/research/pesde.md` (copied from the investigation); the CLAUDE/AGENTS-level facts it pins: no MSRV or library-stability promise, 0.8 (unreleased HEAD) already breaks `Project::new`; `indices` is `skip_serializing` so manifest edits must be textual (`toml_edit`); git sources and the registry index use gix (pure Rust), tarballs use reqwest — the `patches` feature is the only git2/libgit2 consumer.

## Goals / Non-Goals

Goals: everything in the proposal/specs — the `ptah package` group, `.ptah/`-rooted Pesde layout, init scaffolding, `@alias` requires through a ptah-synced root `.luaurc`, offline e2e coverage.

Non-Goals (beyond the proposal's): rewriting factory-components as packages (they keep mounting until a publishing change); any port in `ptah-core` (the port set stays closed); Windows-specific hard-link fallbacks beyond what pesde already does; package scripts (`[scripts]`), patches, wally sources, workspaces.

## Decisions

### D1 — New adapter crate `crates/ptah-pesde`, no core port
Follows the `ptah-config` pattern: a small I/O adapter composed from `ptah-cli`, depending on `ptah-core` (errors only) + `pesde` + `toml_edit` + `tokio` + `reqwest`. The domain crate never learns about packages; `deps_guard` is untouched (it only scans `ptah-core`). Alternatives: putting the code in `ptah-cli` (rejected — cli.rs is already 1.1k lines and the driver is a coherent unit) or a `PackageSource` port in core (rejected — new ports are their own design decision per AGENTS.md; nothing in core needs the abstraction).

### D2 — Pin `pesde = "=0.7.4"`, `default-features = false`
Drops `wally-compat` (serde_json passthrough we don't use) and `patches` (the only libgit2/C dep; nobody patches ptah packages). Git sources (e.g. `github.com/patextreme/ptah-libs`) and registry tarballs are unaffected — gix over reqwest/rust-tls. Exact pin because pesde is 0.x with breaking releases and no stability promise; upgrading (0.8 breaks `Project::new`) is a deliberate adapter-scoped change. The adapter isolates every pesde type behind its own API so a bump touches one crate.

### D3 — Project roots and caches
`Project::new(package_dir = <project>/.ptah, workspace_dir = None, data_dir = <project>/.ptah/.pesde/data, cas_dir = <project>/.ptah/.pesde/cas, AuthConfig::new())`. Project discovery: walk up from the invocation dir for a directory containing `.ptah/config.toml` or `.ptah/pesde.toml` (generalizing `ptah-config::find_project_config`, which only looks for `config.toml`). Per-project caches (not `~/.pesde`) keep ptah invisible to a user's own pesde install and make the sandbox/nix story simple; CAS sits under `.ptah` so the same-filesystem hard-link constraint holds trivially. Trade-off: no cross-project cache sharing (bigger disks, re-downloads across projects) — acceptable, revisit if users complain; a future `data_dir` under `$XDG_CACHE_HOME` is a config knob away.

### D4 — Default index supplied in-memory, manifest stays textual
The skeleton carries no `[indices]`. At resolve time the adapter deserializes the manifest and, when `indices` is empty, injects `default = <ptah's compiled-in default index URL>` (pesde's own default index) into the in-memory `Manifest` before resolution; writes never go through `Manifest` serialization (`indices` is `skip_serializing` anyway) but through `toml_edit` on the raw text, so user `[indices]` entries survive byte-for-byte. Default URL is a `const` in `ptah-pesde` — one-line change if ptah ever hosts its own index.

### D5 — Driver layer reimplemented from pesde's CLI (~200 lines)
`add` (resolve newest via `PackageSource::resolve` + `versions.pop_last()`, insert `^<version>` via `toml_edit`, then install), `remove` (toml_edit delete, then install), `install` (reuse lockfile graph via `dependency_graph(Some(&old))`, `download_and_link`, build + write `Lockfile`), `install --locked` (reimplement `up_to_date_lockfile`: bail when lockfile missing, or manifest name/target/overrides/direct specs don't match), `update` (`dependency_graph(None, ..)`). All async on the tokio runtime the CLI already creates for `run`; package commands get their own small runtime block like `run`'s. Reporters: pesde's `()` no-op impls for now — ptah prints one line per completed stage (resolved / downloaded / linked) rather than a progress bar.

### D6 — `.luaurc` sync as a derived artifact
`ptah package add|remove|install|update` recompute the alias set from the manifest's dependency tables and sync the project-root `.luaurc`: create when absent; parse (serde_json), replace only entries in `aliases` whose values point into `<project>/.ptah/luau_packages/`, preserve everything else; a user entry colliding with a package alias is a diagnostic, user entry wins, the package alias is skipped (its require will fail loudly). Format: standard `{"aliases": {"<alias>": ".ptah/luau_packages/<alias>"}}`. Treated like `ptah.d.luau`: ptah-synced, user-committed. Alternatives: generating `.ptah/.luaurc` (rejected — Luau's config discovery walks up from the requiring file; workflows outside `.ptah/` would never find it) or stateless `to_alias_override` in mlua (rejected in the grill: editors/luau-lsp need a real file anyway, and one file keeps runtime/check/editors in agreement).

### D7 — Runtime alias resolution via mlua's native config walk
`ScriptRequier` flips `has_config()` to true (a `.luaurc`/`.config.luau` exists in the current directory) and `config()` returns its bytes — mlua then performs the walk-up and JSON parsing itself and calls `jump_to_alias` with the resolved path, which we land like any other path. Standard Luau semantics for free; no ptah-specific alias table in the runtime. `ptah-check`'s lint walker duplicates the walk-up purely (walk parents from the requiring file, parse `aliases`, resolve relative to the config's directory), the same deliberate duplication pattern as today's `resolve_edge`. The luau-lsp pass: verify `luau-lsp analyze` discovers the root `.luaurc` from the entry's path (expected, it's standard platform behavior); if not, pass the project `.luaurc` via `--base-luaurc` (task includes this verification spike).

### D8 — Offline test fixtures: local git index + loopback archive server
Following the `mock-agent` precedent: `crates/ptah-cli/tests/fixtures/pkg/` holds fixture packages; a test helper builds a local git index repository (format per pesde's index spec) and serves archives from an in-process loopback HTTP server (tiny `tokio`/`hyper` or `std::net` listener serving gzipped tarballs) plus a local bare git repo for the `--git` path. AGENTS.md's "fully offline" invariant gets a clarifying phrase: "no external network; loopback test doubles (mock-agent, fixture registry) are the established pattern."

### D9 — Init writes the manifest skeleton from a pinned const
`PESDE_SKELETON` in cli.rs next to `CONFIG_SKELETON` (name `components/<lowercased-cwd-name>` sanitized to valid scope/name chars, `private = true`, `[target] environment = "luau"`). Same ownership rule as config.toml: skip-with-message when present. Init performs no resolution, no network, no lockfile write, no `.luaurc` write (no deps → no aliases; the file appears with the first package command).

## Risks / Trade-offs

- [pesde 0.x churn, no stability promise; 0.8 already breaks `Project::new`] → exact pin `=0.7.4`, every pesde type confined to `ptah-pesde`, upgrade is one crate's problem. Documented in D2.
- [Loopback test server drift from pesde's real index/archive formats] → fixtures built by the same pesde library calls the real index uses where possible (e.g. lockfile/graph types); format changes surface as compile breaks, not silent drift.
- [`.luaurc` merge corrupting user files] → sync is parse-modify-serialize over JSON with preservation of unknown keys, plus a unit-test matrix (absent file, user keys, user aliases, collisions); never a blind overwrite.
- [Hard-link CAS across filesystems (`.ptah` on a different mount than a bind-mounted project)] → CAS lives inside `.ptah/.pesde` itself; the failure mode pesde guards against can't arise.
- [luau-lsp analyze may not auto-discover the root `.luaurc` from an entry path] → D7 verification spike early in implementation; fallback `--base-luaurc` is a one-flag change and is spec-compatible either way.
- [Registry index format is a git repo owned by pesde's index tooling] → tests generate the minimal index shape pesde's `read_index_file` consumes; a format break is caught by the same pin discipline as D2.

## Migration Plan

Purely additive: no existing command changes behavior except `init` (one more scaffold file, only when absent — re-running in existing projects adds `pesde.toml` on the next init, which is the designed completion behavior, matching how partial scaffolds complete today). Rollback = remove the `package` command group; existing projects keep working because `run`/`check` semantics are unchanged for non-alias requires.

## Open Questions

- Exact fixture-registry helper placement (`ptah-pesde` tests vs shared `ptah-cli` test-util module) — decided during implementation by where the e2e test lands.
- Whether `ptah package` output should eventually grow structured progress (reporter traits are already injectable) — deferred until there's a TUI/verbose-mode consumer.
