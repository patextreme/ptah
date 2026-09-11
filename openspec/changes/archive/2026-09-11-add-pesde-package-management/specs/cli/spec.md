## MODIFIED Requirements

### Requirement: Init subcommand scaffolds a project .ptah directory
The CLI SHALL provide `ptah init`, which scaffolds `./.ptah/` relative to the current working directory with exactly three files:

- `.ptah/ptah.d.luau` — the project definitions, byte-identical to `ptah types` standard output (version header included) whenever init writes them;
- `.ptah/config.toml` — a fully commented skeleton documenting the two-layer registry discovery (project entries override user entries per agent name), `${VAR}` environment interpolation, and the per-agent fields (`command` required, `args` and `env` optional), which SHALL parse as a valid empty registry exactly as written;
- `.ptah/pesde.toml` — a package-manifest skeleton which SHALL parse as a valid private Pesde manifest targeting the `luau` environment, named `components/<lowercased-current-directory-name>`, with no dependency entries and no `[indices]` table.

The three files have different ownership and different existing-file semantics. `.ptah/config.toml` and `.ptah/pesde.toml` are user-authored: an existing file SHALL be skipped with a per-file skipped message and never modified. `.ptah/ptah.d.luau` is a derived artifact of the installed binary: init SHALL sync it — created when absent, overwritten with the current binary's emitted definitions whenever its bytes differ from the current output in either accepted form (the emitted output itself, or the emitted output without its header line), and left untouched only when it already matches one of those forms byte-for-byte. The headerless arm exists so a file laid out like the repository's source definitions (the headerless compile input) reports as current instead of being rewritten with a prepended header. Overwrite is otherwise unconditional: a differing file with no parseable ptah version header (hand-edited or foreign) SHALL still be overwritten.

`ptah init` SHALL NOT create any other files (no starter script, no editor configuration), SHALL NOT search parent directories for an existing `.ptah`, SHALL NOT write to the user-level config directory, and SHALL NOT perform package installation: init writes no `pesde.lock`, no `luau_packages/` content, and no root `.luaurc`, and makes no network requests. Running `ptah init` twice with the same binary SHALL change no file: the configs are skipped and the definitions already match. On success the command SHALL print exactly one line per file — `created:` or `skipped (exists):` for the configs; `created:`, `updated:`, or `up to date:` for the definitions — followed by next-step hints (editing the registry, pointing luau-lsp at the definitions, installing shell completions, adding packages, the ptah skill) to standard output, and exit 0. An `updated:` definitions line SHALL carry the previous and current version in parentheses when the overwritten file's first line parsed as a ptah version header, and no version suffix otherwise. The hints SHALL print on every run, including runs where no file changed. A failure to write (for example an unwritable directory) SHALL print an error to standard error and exit 1.

#### Scenario: Fresh init creates both files
- **WHEN** `ptah init` runs in a directory with no `.ptah`
- **THEN** `.ptah/ptah.d.luau`, `.ptah/config.toml`, and `.ptah/pesde.toml` exist, the process exits 0, and each created file is announced on standard output

#### Scenario: Written definitions match the installed binary
- **WHEN** `.ptah/ptah.d.luau` written by `ptah init` is compared with `ptah types` output
- **THEN** they are byte-identical

#### Scenario: Skeleton is a valid empty registry
- **WHEN** `.ptah/config.toml` written by `ptah init` is parsed as a registry
- **THEN** it parses without error and contains no agents

#### Scenario: Manifest skeleton is a valid package manifest
- **WHEN** `.ptah/pesde.toml` written by `ptah init` is parsed as a Pesde manifest
- **THEN** it parses without error, is private, targets the `luau` environment, and contains no dependencies or indices

#### Scenario: Re-running init is idempotent
- **WHEN** `ptah init` runs a second time in the same directory with the same binary
- **THEN** the configs are reported as skipped (exists), the definitions are reported as up to date, no file's bytes change, and the process exits 0

#### Scenario: Stale definitions are updated
- **WHEN** `ptah init` runs where `.ptah/ptah.d.luau` was written by an older binary (content differs, first line is a ptah version header naming an older version)
- **THEN** the file is overwritten with the current emitted definitions, the line on standard output reports the update carrying both versions, and the process exits 0

#### Scenario: Modified or foreign definitions are overwritten
- **WHEN** `ptah init` runs where `.ptah/ptah.d.luau` differs from the current output and its first line is not a ptah version header (hand-edited or foreign content)
- **THEN** the file is overwritten with the current emitted definitions, the update line carries no version suffix, and the process exits 0

#### Scenario: Source-layout definitions are left untouched
- **WHEN** `ptah init` runs where `.ptah/ptah.d.luau` is byte-identical to the current emitted output without its header line (the repository's own source-definitions file)
- **THEN** the file is not written, it is reported as up to date, and the process exits 0

#### Scenario: Partial scaffold completes
- **WHEN** `ptah init` runs in a directory where `.ptah/config.toml` and `.ptah/pesde.toml` already exist but `.ptah/ptah.d.luau` does not
- **THEN** the definitions file is created, the existing configs are neither modified nor clobbered, and the process exits 0

#### Scenario: Init stays offline
- **WHEN** `ptah init` runs with no network available
- **THEN** it completes successfully having written only the three scaffold files

#### Scenario: Hints print on every run
- **WHEN** `ptah init` completes, whether files were created, updated, or unchanged
- **THEN** the next-step hints appear on standard output

#### Scenario: Unwritable target fails cleanly
- **WHEN** `ptah init` cannot create `./.ptah` (for example a read-only parent directory)
- **THEN** an error is printed to standard error and the process exits 1

### Requirement: Completions subcommand emits shell completion scripts
The CLI SHALL provide `ptah completions <shell>`, where `<shell>` is a required positional argument accepting exactly `bash`, `zsh`, `fish`, `elvish`, and `powershell`. The command SHALL print the completion script for the named shell to standard output and SHALL print nothing else. Emitted scripts SHALL be generated from the binary's own command tree, so completions always match the installed binary's surface; hidden subcommands SHALL NOT appear in the emitted scripts. A missing or unknown `<shell>` argument SHALL be a usage error printing to standard error and exiting 2. The command SHALL NOT require a script, registry, or agent configuration and SHALL NOT touch the filesystem. The README SHALL document per-shell installation lines covering at least bash, zsh, and fish.

#### Scenario: Emit a bash completion script
- **WHEN** `ptah completions bash` is invoked
- **THEN** standard output contains a bash completion script registering a completion handler for `ptah`, and the process exits 0

#### Scenario: Every supported shell emits a plausible script
- **WHEN** `ptah completions <shell>` is invoked for each of `zsh`, `fish`, `elvish`, and `powershell`
- **THEN** each invocation prints a non-empty, shell-appropriate script to standard output and exits 0

#### Scenario: Visible subcommands appear, hidden ones do not
- **WHEN** a completion script is generated for any shell
- **THEN** the visible subcommands (`run`, `check`, `types`, `completions`, `init`, `package`) appear in it and the hidden `__bridge` subcommand does not

#### Scenario: Unknown shell is a usage error
- **WHEN** `ptah completions tcsh` is invoked
- **THEN** a usage error is printed to standard error and the process exits 2

#### Scenario: No side effects
- **WHEN** `ptah completions fish` runs on a machine with no agent registry and no `.ptah` directory
- **THEN** it exits 0 without creating files or spawning agents
