## MODIFIED Requirements

### Requirement: Relative module resolution
Scripts SHALL be able to `require` modules by relative path from the requiring file's directory (e.g. `require("./lib/pipeline")`, resolving `.luau` files). Relative paths resolve without a boundary: a require MAY traverse out of the entry script's directory (e.g. `require("../shared/helper")`) to any module reachable by relative path.

Scripts SHALL also be able to `require` modules by alias (`require("@hello")`): an alias require resolves by walking upward from the requiring file's directory to the nearest `.luaurc` (or `.config.luau`) configuration file with an `aliases` table, exactly matching Luau's native require-by-string alias semantics; the alias's target resolves like a relative module path anchored at the configuration file's directory. Remaining path segments after the alias append to the target. Require strings that are neither relative nor alias form (absolute paths, bare module names) MUST be rejected with a Lua error; an alias missing from the discovered configuration, or an alias require in a file with no discoverable configuration, raises the same rejection error.

#### Scenario: Sibling module
- **WHEN** a script at `main.luau` requires `./lib/util` and `lib/util.luau` exists
- **THEN** the module is loaded and its return value provided; a second require of the same path returns the cached module

#### Scenario: Module outside the entry script's directory
- **WHEN** a script at `workflow-1/main.luau` requires `../shared/helper` and `shared/helper.luau` exists as a sibling of `workflow-1/`
- **THEN** the module is loaded and its return value provided exactly as an in-directory module would be

#### Scenario: Alias require resolves an installed package
- **WHEN** a project's root `.luaurc` maps alias `hello` into `.ptah/luau_packages/hello`, the entry requires `@hello`, and the target module exists
- **THEN** the module is loaded and its return value provided; a second require returns the cached module

#### Scenario: Alias configuration is discovered upward
- **WHEN** a script at `<project>/.ptah/workflows/x/main.luau` requires `@hello` and the nearest alias configuration is `<project>/.luaurc`
- **THEN** the alias resolves against that file's directory

#### Scenario: Missing module
- **WHEN** a script requires a path that does not resolve to an existing `.luau` file
- **THEN** the require call raises a Lua error naming the unresolved path

#### Scenario: Non-relative require string rejected
- **WHEN** a script requires an absolute path (`require("/etc/x")`) or a bare module name (`require("shared/helper")`)
- **THEN** the require call raises a Lua error stating that only `./`, `../`, and `@alias` paths are allowed

#### Scenario: Unknown alias rejected
- **WHEN** a script requires `@nope` and the discovered alias configuration defines no `nope` alias
- **THEN** the require call raises a Lua error naming the unresolved alias
