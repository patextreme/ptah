## Why

The repository's own Luau surface — `examples/`, the `factory-components/`
library, the `.ptah/workflows/` shims, the test fixtures — is type-gated in
place only by raw `luau-lsp` (`checks.ptah-analyze`) and behavior-gated by
cargo tests that check *copies* or self-written scripts. Nothing enforces
formatting (style drift is invisible until review), nothing runs `ptah check`
itself over the actual repo tree (require-resolution and registry regressions
in the real layout are caught only transitively), and `AGENTS.md` gives
contributors no formatting or check instruction at all. The tooling to close
all three gaps already exists; only the wiring is missing.

## What Changes

- **StyLua formatting gate**: `stylua --check .` folded into the existing
  `checks.ptah-analyze` derivation's `checkPhase`. StyLua **defaults** are
  the house style (the tree is already tab-indented, double-quoted,
  always-parenthesized); no `stylua.toml` is committed — the version is
  pinned by the locked nixpkgs `pkgs.stylua`.
- **`.styluaignore`** (new, one line: `.ptah/ptah.d.luau`) so a bare
  `stylua .` can never touch the generated definitions file, whose
  byte-identity with `ptah types` output is a standing contract.
- **One-time `stylua .` reformat** of every tracked `.luau` file; expected
  to be a small diff (defaults already match), no semantic changes, covered
  by the existing example/factory-component tests.
- **In-place `ptah check` gate**: a new nix check running the **release
  package** (`config.packages.ptah` — the shipped `ptah` binary with its
  same-commit embedded definitions, plus its bundled `mock-agent`) over the
  entry scripts `examples/*.luau`, `examples/*/*.luau`, and
  `.ptah/workflows/*/main.luau` (the shims pull the whole
  `factory-components/` require graph in transitively; no new probe file).
  Registry: `HOME`-based user-level config defining `demo` and `pi` →
  mock-agent, because the sandbox source deliberately strips
  `.ptah/config.toml`. Zero-execution — nothing spawns.
- **Devshell** gains `pkgs.stylua` so the contributor instruction is
  runnable in `nix develop`.
- **`AGENTS.md`**: two lines under Commands (`stylua .`; `ptah check
  <script>` on `.luau` edits) plus a Testing bullet noting the nix gates
  enforce both.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `type-definitions`: adds a requirement pinning the repository's Luau
  script gates (formatter check + shipped-binary in-place `ptah check` over
  the bundled entry scripts) as part of the repo's check suite, and amends
  the "Editor setup documentation" committed-config exception sentence to
  name the contributor-facing `.styluaignore` (StyLua defaults; no
  `stylua.toml`).
- `script-checking`: amends the "Check documentation" requirement — the
  repository's agent instructions SHALL additionally direct contributors to
  format `.luau` changes with StyLua and run `ptah check` on them.

## Impact

- `nix/checks.nix` — `ptah-analyze` gains the `stylua --check` step; new
  `ptah-check` derivation (release binary + synthesized HOME registry).
- `nix/devshell.nix` — `pkgs.stylua`.
- `.styluaignore` — new file at repo root.
- ~20 tracked `.luau` files reformatted once (examples, factory-components,
  workflows, fixtures). `.ptah/ptah.d.luau` untouched (ignored).
- `AGENTS.md` — Commands + Testing additions.
- No Rust code changes, no CLI behavior changes, no consumer-facing surface
  changes. Local-run nuance: untracked `.ptah/workflows/adhoc/` is swept
  into local `nix flake check` sources (pre-existing quirk of the
  neighboring gate); benign — it is strict, `pi`-based, and covered by the
  synthesized registry.
