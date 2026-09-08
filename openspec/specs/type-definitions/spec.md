# Type Definitions Specification

## Purpose

Defines the Luau type definitions that describe the `ptah` script API and its sandboxed environment to editors and analyzers: their content contract, distribution via the `ptah types` subcommand, and the guards that keep them synchronized with the runtime.

## Requirements

### Requirement: Definitions cover the script API
A definitions file SHALL declare the `ptah` global with its full public surface: `agent`, `spawn`, `parallel`, `join`, `sleep`, `log`, `exit`, and `version`; session objects (`prompt` returning a result table with `text`, `stopReason`, a `usage` table of `input`/`cacheRead`/`cacheWrite`/`output`, and `result` holding the turn's typed-result value (`nil` when there was no accepted submission), `cancel`, `label`, `close`, `configOptions`, and `setConfig`); task objects (`await`); session and task option tables; and agent spec tables. Outcome entries SHALL be typed as a discriminated union of `{ ok: true, value: T } | { ok: false, error: string }`, and `parallel`/`spawn` SHALL be generic so result types propagate. The `mcpServers` option SHALL be typed after the session-configuration structure the runtime accepts, not left untyped; the `resultSchema` session option SHALL be typed as an optional string-keyed table carrying the declared JSON Schema and the prompt-result `result` field as an optional field for the converted submission value. The `SessionOptions` type SHALL NOT declare a `config` field (the option is removed; scripts apply config with `setConfig` after session creation). The config-option surface SHALL be typed: `configOptions()` returning an array of option entries (`id`, `name`, `type`, `currentValue: string | boolean`, optional `category`, and an `options` choice array for select options) and `setConfig(id: string, value: string | boolean)`.

The definitions SHALL additionally type `exec`: `ptah.exec(cmd: string, opts?: { timeoutMs: number? }) -> ExecResult` where `ExecResult` is `{ exitCode: number, stdout: string, stderr: string }`. The definitions SHALL additionally type the `json` module: `ptah.json.parse(s: string) -> any` (raising on malformed input) and `ptah.json.stringify(value: any, opts?: { indent: number? }) -> string`.

#### Scenario: Typo in result field
- **WHEN** a script analyzed with the definitions accesses an invented field on a prompt result (e.g. `r.txt`)
- **THEN** analysis reports a type error naming the result table type

#### Scenario: Outcome narrowing
- **WHEN** a script binds a `ptah.parallel` result to a local and branches on `entry.ok`
- **THEN** analysis narrows the local to the `value` field on the true branch and the `error` field on the false branch

#### Scenario: Typed-result surface type-checks
- **WHEN** a strict-mode script analyzed with the definitions passes `resultSchema = { type = "object" }` in `agent:session(…)` options and reads `r.result` on a prompt outcome
- **THEN** analysis accepts both uses, while an invented outcome field (e.g. `r.txt`) still reports a type error naming the result table type (excess keys in option table literals are a known analyzer residual, documented in the README)

#### Scenario: Constructor config type-checks
- **WHEN** a script analyzed with the definitions passes `config = { model = "opus" }` in `agent:session(…)` options
- **THEN** the `SessionOptions` type declares no `config` field (excess keys in option table literals are a known analyzer residual, documented in the README), and running the script raises the pre-spawn rejection error instead

#### Scenario: Wrong setConfig value type
- **WHEN** a script analyzed with the definitions calls `s:setConfig("model", 42)`
- **THEN** analysis reports a type error on the value argument

#### Scenario: Exec result fields type-check
- **WHEN** a script analyzed with the definitions binds `local r = ptah.exec("true")` and reads `r.exitCode`, `r.stdout`, `r.stderr`, then reads `r.out`
- **THEN** the first three reads are accepted and `r.out` reports a type error naming the exec result type

#### Scenario: Exec options type-check
- **WHEN** a strict-mode script analyzed with the definitions calls `ptah.exec("true", { timeoutMs = 100 })` and separately `ptah.exec(cmd, 100)`
- **THEN** the options-table call is accepted and the bare-number call reports a type error

#### Scenario: JSON module type-checks
- **WHEN** a script analyzed with the definitions calls `ptah.json.parse(s).x` and `ptah.json.stringify(v, { indent = 2 })`
- **THEN** both calls are accepted, and a call to an invented member (e.g. `ptah.json.load`) reports a type error

### Requirement: Definitions model the sandbox
The definitions SHALL shadow the trimmed globals the runtime provides: `os` restricted to `time` and `clock`, `coroutine` restricted to `yield`, and `loadstring` and `collectgarbage` declared as nil.

#### Scenario: Removed global flagged
- **WHEN** a script analyzed with the definitions calls `os.date`, `coroutine.create`, or `loadstring`
- **THEN** analysis reports a type error instead of the call being accepted and failing at runtime

### Requirement: Types subcommand
The CLI SHALL provide `ptah types`, which prints the definitions to standard output prefixed with a generated header comment identifying the ptah version. The emitted definitions SHALL be byte-identical to the repository's definitions file apart from the header. The command SHALL exit 0 and not require a script, registry, or agent configuration.

#### Scenario: Emit definitions
- **WHEN** a user runs `ptah types`
- **THEN** the definitions are printed to stdout with a version header, suitable for redirection into a definitions file

#### Scenario: No side effects
- **WHEN** `ptah types` runs on a machine with no agent registry configured
- **THEN** it succeeds without spawning agents or reading script files

### Requirement: Definitions stay synchronized with the runtime
The repository SHALL include a runtime probe test that executes a script (against the mock agent) exercising every member, method, and field the definitions promise. The repository's check suite SHALL run static analysis over the bundled examples, the probe script, and script test fixtures using the definitions, in strict mode via per-file directives rather than a committed `.luaurc`.

#### Scenario: Defs promise a removed member
- **WHEN** a member documented in the definitions is removed or renamed in the runtime
- **THEN** the probe test fails

#### Scenario: Example regresses
- **WHEN** a bundled example or fixture contains a type error against the definitions
- **THEN** the static-analysis check fails

### Requirement: Editor setup documentation
The README SHALL document `ptah init` as the front door for obtaining and refreshing definitions: it scaffolds a commented `.ptah/config.toml` registry skeleton and writes or updates `.ptah/ptah.d.luau` (byte-identical to `ptah types` output) into `./.ptah/` in the working directory. The README SHALL document re-running `ptah init` after upgrading the binary as the primary way to refresh the definitions, and `ptah types > .ptah/ptah.d.luau` as the documented alternative (for scripting, or refreshing without touching config), plus the generic luau-lsp settings (VS Code and Neovim, standard platform) pointing at `.ptah/ptah.d.luau`. The repository SHALL NOT commit editor or Luau configuration files aimed at consumers; the contributor-facing `.helix/languages.toml`, which points luau-lsp at `.ptah/ptah.d.luau` for files in this repository, is the settled exception. The documentation SHALL note the known residuals: strict analysis of generic `map` callbacks occasionally needs explicit parameter annotations; the prompt-result string-conversion sugar is not covered; outcome narrowing requires a local binding.

#### Scenario: Reader configures an editor
- **WHEN** a reader follows the README editor-setup section
- **THEN** they can produce a definitions file matching their installed ptah version (via `ptah init`, refreshed after an upgrade by re-running `ptah init` or via `ptah types > .ptah/ptah.d.luau`) and point luau-lsp at `.ptah/ptah.d.luau` using documented generic settings

#### Scenario: Reader understands the require-tree residual
- **WHEN** a reader encounters the residuals list in the editor-setup section
- **THEN** it contains no require-tree entry; the documentation states that editor analysis and ptah resolve relative requires identically
