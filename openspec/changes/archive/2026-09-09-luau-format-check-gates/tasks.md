## 1. Formatting groundwork

- [x] 1.1 Add `pkgs.stylua` to `nix/devshell.nix` packages. Verify: entering `nix develop` and running `stylua --version` prints a version.
- [x] 1.2 Create `.styluaignore` at the repo root containing exactly `.ptah/ptah.d.luau`. Verify: `stylua --check .` from the repo root (in the dev shell) reports no diff touching `.ptah/ptah.d.luau` even after the reformat in 1.3.
- [x] 1.3 One-time reformat: run `stylua .` from the repo root inside `nix develop`. Verify: `git diff --stat` shows only `.luau` files under `examples/`, `factory-components/`, `.ptah/workflows/`, and `crates/ptah-cli/tests/fixtures/`; `.ptah/ptah.d.luau` untouched (`git diff --exit-code .ptah/ptah.d.luau`); `stylua --check .` now exits 0.

## 2. Formatter gate

- [x] 2.1 Extend `checks.ptah-analyze` in `nix/checks.nix`: add `pkgs.stylua` to `nativeBuildInputs` and a `stylua --check .` line in `checkPhase` (before or after the existing `luau-lsp analyze` — both run; either failing fails the derivation). Verify: introduce a deliberate formatting error in a scratch copy and confirm the derivation fails on the stylua step; revert.
- [x] 2.2 Negative-path sanity in situ: temporarily de-indent a tracked example, run `nix build .#checks.x86_64-linux.ptah-analyze` (or `nix flake check` scoped if cheaper), confirm failure names the file, then `stylua .` again to restore. Verify: gate green after restore.

## 3. In-place `ptah check` gate

- [x] 3.1 Add a new derivation in `nix/checks.nix` (e.g. `checks.ptah-check`): `src = config.ptahSrc`; `nativeBuildInputs = [config.packages.ptah pkgs.luau-lsp]` — `ptah check`'s typecheck pass PATH-discovers `luau-lsp` and exits `2` when it is absent, and the release package does not ship it (same pattern as `checks.ptah-tests`); `checkPhase` creates `$HOME/.config/ptah/config.toml` defining `demo` and `pi` → `${config.packages.ptah}/bin/mock-agent`, then loops over `examples/*.luau`, `examples/*/*.luau`, `.ptah/workflows/*/main.luau`, running `"${config.packages.ptah}/bin/ptah" check` once per entry (the command takes exactly one positional script) with `HOME` and `XDG_CONFIG_HOME` pinned so discovery finds only the synthesized registry; any non-zero exit fails. Verify: derivation builds green via `nix flake check`.
- [x] 3.2 Confirm registry fallthrough: the unpacked source has no `.ptah/config.toml` (source filter strips it), so discovery reaches the HOME registry only. Verify: run the derivation's script body manually in a temp dir against the store source and observe `ptah check` exits 0 for all entries; also confirm zero subprocess spawns (mock-agent never logs).

## 4. Documentation

- [x] 4.1 `AGENTS.md`: under Commands add `stylua .` (format Luau; gate enforces StyLua defaults) and a line directing `ptah check <script>` on `.luau` edits; under Testing add a bullet noting `nix flake check` enforces both the formatter gate and the in-place `ptah check` gate over the repo's own scripts. Verify: reading AGENTS.md, a contributor knows the two commands and that the sandbox enforces them.
- [x] 4.2 Check README consistency: the type-definitions spec's committed-config exception sentence now names `.styluaignore`; confirm README's editor-setup section does not claim the repo commits *no* Luau config beyond `.helix/` (adjust wording if it enumerates). Verify: `rg -n "helix|stylua" README.md` shows no contradiction with the amended spec.

## 5. Verification

- [x] 5.1 Full suite in the dev shell: `nix develop -c bash -c 'cargo test'` green after the reformat (example and factory-component tests re-cover churned files). Verify: exit 0.
- [x] 5.2 `nix flake check` green in the sandbox (both gates + existing checks). Verify: exit 0.
- [x] 5.3 `openspec validate luau-format-check-gates --strict` passes. Verify: exit 0.
