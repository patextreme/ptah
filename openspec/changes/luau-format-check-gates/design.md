# Design: Luau format + check gates

## Context

See `proposal.md` for motivation. Facts that constrain the design:

- `checks.ptah-analyze` (`nix/checks.nix`) already runs raw `luau-lsp
  analyze` in place over the repo's Luau globs with `pkgs.luau-lsp`; it is
  the natural home for a sibling formatter step covering the same surface.
- `nix/source.nix`'s filter keeps `.ptah/ptah.d.luau`, `.ptah/workflows/**/
  *.luau`, and `factory-components/` in the sandbox source — but strips
  `.ptah/config.toml`. So in-place `ptah check` in the sandbox sees **no
  project registry**, deterministically (the filter applies to the working
  tree copy, so local runs match CI).
- `config.packages.ptah` (release package) ships both `ptah` and
  `mock-agent` in its `/bin`, and embeds the same commit's
  `.ptah/ptah.d.luau` via `include_str!`. `checks.ptah-smoke` already
  demonstrates the release-binary + synthesized-registry pattern.
- Every tracked `.luau` file is already tab-indented, double-quoted,
  always-parenthesized — i.e. StyLua's defaults match the house style.
- Scripts reference exactly two agent names: `demo` (all examples) and
  `pi` (workflow shims). Workflow shims require
  `../../../factory-components/...`, so entries must be checked where they
  lie; no copying.
- `.ptah/ptah.d.luau` is byte-identical to `ptah types` output (pinned by
  the `init`/`types` sync tests); a formatter must never rewrite it.

## Goals / Non-Goals

Goals: mechanical enforcement of the two contributor rules (format with
StyLua, run `ptah check`) at `nix flake check` time, using the artifacts we
ship.

Non-Goals: no `stylua.toml` (defaults are the style; the locked nixpkgs
`pkgs.stylua` is the version pin); no editor integration for formatting; no
`ptah format` subcommand; no change to `ptah check` itself; no CI beyond the
existing `nix flake check` surface.

## Decisions

1. **StyLua defaults, no config file; `.styluaignore` only.** The tree
   already conforms to defaults, so the one-time reformat is minimal and
   the diff reviewable. Reproducibility across contributor machines comes
   from the nixpkgs lock (devshell + gate use the same `pkgs.stylua`),
   not from a written-down style sheet. `.styluaignore` exists because the
   *generated* definitions file must be exempt — without it, a bare
   `stylua .` (the instruction we document) would corrupt the
   byte-identity contract. *Alternative rejected*: commit `stylua.toml`
   freezing defaults — one more config file to keep honest, and the spec's
   committed-config exception list grows for no behavioral gain.

2. **Fold `stylua --check .` into `checks.ptah-analyze`.** One derivation
   already owns "Luau-surface honesty in the sandbox" over the same file
   set; StyLua's output names offending files clearly, so failure
   attribution does not suffer; one less derivation to build.
   *Alternative rejected*: separate `checks.ptah-format` — only worth it
   if the two gates needed different sources or native inputs.

3. **New derivation for in-place `ptah check`, driven by the release
   package.** The repo *is* ptah, so the gate validates the repo's scripts
   with the artifact we ship (`config.packages.ptah`'s `ptah` + bundled
   `mock-agent`; embedded defs come from the same commit — no skew).
   Dev-binary behavior is already covered by `checks.ptah-tests`
   (`PTAH_REQUIRE_REAL_LSP=1`). *Alternative rejected*: a cargo-test
   harness gate — cargo tests already check *copies* (factory_components
   mounts `vendor/factory-components`) and self-written scripts; the
   in-place real-layout coverage is exactly what's missing.

4. **HOME-based synthesized registry, not a source-tree config.** The
   sandbox source has no `.ptah/config.toml` (by the source filter's
   design), and writing one into the unpacked store path is impossible.
   Instead the derivation sets `HOME` to a temp dir containing
   `.config/ptah/config.toml` defining `demo` and `pi` → the release
   package's `mock-agent`. Discovery walks up from the invocation dir
   (finds nothing — same as designed), then falls to the user registry.
   `ptah check` is zero-execution, so the registry entry is lint-resolution
   only. Entries checked: `examples/*.luau`, `examples/*/*.luau`,
   `.ptah/workflows/*/main.luau` — the shims pull the whole
   `factory-components/` require graph transitively, so no probe file is
   needed. *Alternative rejected*: copy the tree to a writable temp dir and
   drop a project-level config — more code, and checks would no longer run
   against the real layout that ships.

5. **One-time `stylua .` reformat inside this change.** Mechanical, no
   semantic changes; the example and factory-component tests re-cover the
   churned files; the new gates prove the tree conformant.

## Risks / Trade-offs

- [StyLua defaults drift across major versions] → the gate and devshell
  pin one `pkgs.stylua` from the locked nixpkgs; a bump that changes
  defaults surfaces as a (reviewable) reformat, not silent divergence.
- [Untracked `.ptah/workflows/adhoc/` sweeps into local `nix flake check`
  sources but not fresh checkouts] → pre-existing quirk of the neighboring
  luau-lsp glob, not introduced here; the file is strict, `pi`-based, and
  covered by the synthesized registry. Documented, not fixed (tightening
  the source filter to git-tracked-only is out of scope).
- [Formatter churn touches test-covered shims] → tests re-run as part of
  the change's verification; any behavioral break (there should be none —
  StyLua is semantics-preserving) is caught before archive.
- [`ptah check` exit `2` (could not run) silently passing as "clean" if
  wired wrong] → the derivation's script treats any non-zero exit as
  failure; the smoke-check pattern (explicit binary paths) is reused.

## Migration Plan

Add gates → reformat → verify (`nix flake check` locally, full cargo suite
in the dev shell) → document in `AGENTS.md`. Rollback is deleting the two
check additions and `.styluaignore`; the reformat diff can revert
independently (it is cosmetic).

## Open Questions

(none)
