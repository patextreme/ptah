## MODIFIED Requirements

### Requirement: Editor setup documentation
The README SHALL document `ptah init` as the front door for obtaining and refreshing definitions: it scaffolds a commented `.ptah/config.toml` registry skeleton and writes or updates `.ptah/ptah.d.luau` (byte-identical to `ptah types` output) into `./.ptah/` in the working directory. The README SHALL document re-running `ptah init` after upgrading the binary as the primary way to refresh the definitions, and `ptah types > .ptah/ptah.d.luau` as the documented alternative (for scripting, or refreshing without touching config), plus the generic luau-lsp settings (VS Code and Neovim, standard platform) pointing at `.ptah/ptah.d.luau`. The repository SHALL NOT commit editor or Luau configuration files aimed at consumers; the contributor-facing `.helix/languages.toml`, which points luau-lsp at `.ptah/ptah.d.luau` for files in this repository, is the settled exception. The documentation SHALL note the known residuals: strict analysis of generic `map` callbacks occasionally needs explicit parameter annotations; the prompt-result string-conversion sugar is not covered; outcome narrowing requires a local binding.

#### Scenario: Reader configures an editor
- **WHEN** a reader follows the README editor-setup section
- **THEN** they can produce a definitions file matching their installed ptah version (via `ptah init`, refreshed after an upgrade by re-running `ptah init` or via `ptah types > .ptah/ptah.d.luau`) and point luau-lsp at `.ptah/ptah.d.luau` using documented generic settings

#### Scenario: Reader understands the require-tree residual
- **WHEN** a reader encounters the residuals list in the editor-setup section
- **THEN** it contains no require-tree entry; the documentation states that editor analysis and ptah resolve relative requires identically
