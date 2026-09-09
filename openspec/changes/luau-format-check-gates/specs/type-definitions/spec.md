## ADDED Requirements

### Requirement: Repository script gates
The repository's check suite SHALL keep the repository's own Luau tree honest in place. It SHALL run a StyLua conformance check over the repository's tracked Luau sources — bundled examples, the Factory Components library, the repository's own workflow shims, and script test fixtures — using StyLua's defaults as the style (no formatter configuration file committed), with the generated `.ptah/ptah.d.luau` exempt from formatting so its byte-identity with `ptah types` output is preserved. It SHALL additionally run `ptah check` — using the shipped release binary with its embedded definitions — over the bundled entry scripts (the examples and the workflow shims, whose literal require graph covers the Factory Components library transitively) in their real in-repo layout, against a synthesized registry that defines the agent names those scripts reference. Neither gate executes script code or spawns an agent.

#### Scenario: Formatting drift
- **WHEN** a tracked Luau source other than the generated definitions file is edited away from StyLua conformance
- **THEN** the check suite's formatter check fails

#### Scenario: In-place check finding
- **WHEN** a bundled entry script, or any module reachable from it through literal requires, gains a `ptah check` finding (syntax error, unresolvable literal require, literal agent name missing from the synthesized registry, missing `--!strict` directive, or type error)
- **THEN** the check suite's in-place check gate fails

#### Scenario: Generated definitions stay untouched
- **WHEN** the formatter check runs over the repository tree
- **THEN** `.ptah/ptah.d.luau` is ignored, preserving its byte-identity with `ptah types` output

## MODIFIED Requirements

### Requirement: Editor setup documentation
The README SHALL document `ptah init` as the front door for obtaining and refreshing definitions: it scaffolds a commented `.ptah/config.toml` registry skeleton and writes or updates `.ptah/ptah.d.luau` (byte-identical to `ptah types` output) into `./.ptah/` in the working directory. The README SHALL document re-running `ptah init` after upgrading the binary as the primary way to refresh the definitions, and `ptah types > .ptah/ptah.d.luau` as the documented alternative (for scripting, or refreshing without touching config), plus the generic luau-lsp settings (VS Code and Neovim, standard platform) pointing at `.ptah/ptah.d.luau`. The repository SHALL NOT commit editor or Luau configuration files aimed at consumers; the contributor-facing `.helix/languages.toml` (which points luau-lsp at `.ptah/ptah.d.luau` for files in this repository) and `.styluaignore` (which keeps the formatter away from the generated definitions) are the settled exceptions — formatting follows StyLua's defaults, so no `stylua.toml` is committed. The documentation SHALL note the known residuals: strict analysis of generic `map` callbacks occasionally needs explicit parameter annotations; the prompt-result string-conversion sugar is not covered; outcome narrowing requires a local binding.

#### Scenario: Reader configures an editor
- **WHEN** a reader follows the README editor-setup section
- **THEN** they can produce a definitions file matching their installed ptah version (via `ptah init`, refreshed after an upgrade by re-running `ptah init` or via `ptah types > .ptah/ptah.d.luau`) and point luau-lsp at `.ptah/ptah.d.luau` using documented generic settings

#### Scenario: Reader understands the require-tree residual
- **WHEN** a reader encounters the residuals list in the editor-setup section
- **THEN** it contains no require-tree entry; the documentation states that editor analysis and ptah resolve relative requires identically
