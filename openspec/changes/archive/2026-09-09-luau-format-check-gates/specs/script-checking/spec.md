## MODIFIED Requirements

### Requirement: Check documentation
The README SHALL document the `check` subcommand (its passes, the luau-lsp PATH dependency, and the strict-directive requirement) and the exit-code contract; the repository's agent instructions SHALL note the extended exit-code contract (`2` also covers "check could not run" for `check`) and SHALL direct contributors to format Luau changes with StyLua (defaults; `stylua .` from the repository root) and to run `ptah check` on the scripts they edit.

#### Scenario: Reader understands check
- **WHEN** a reader follows the README check section
- **THEN** they know what passes run, that luau-lsp must be installed, and what each exit code means

#### Scenario: Contributor follows agent instructions
- **WHEN** a contributor edits a Luau script in this repository and follows the agent instructions
- **THEN** they format the change with StyLua and run `ptah check` on the edited script before committing
