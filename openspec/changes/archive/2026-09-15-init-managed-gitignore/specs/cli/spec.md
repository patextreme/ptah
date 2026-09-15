## MODIFIED Requirements

### Requirement: Init subcommand scaffolds a project .ptah directory
The CLI SHALL provide `ptah init`, which scaffolds `./.ptah/` relative to the current working directory with exactly four files:

- `.ptah/ptah.d.luau` — the project definitions, byte-identical to `ptah types` standard output (version header included) whenever init writes them;
- `.ptah/config.toml` — a fully commented skeleton documenting the two-layer registry discovery (project entries override user entries per agent name), `${VAR}` environment interpolation, and the per-agent fields (`command` required, `args` and `env` optional), which SHALL parse as a valid empty registry exactly as written;
- `.ptah/pesde.toml` — a package-manifest skeleton which SHALL parse as a valid private Pesde manifest targeting the `luau` environment, named `components/<lowercased-current-directory-name>`, with no dependency entries and no `[indices]` table;
- `.ptah/.gitignore` — a source-control ignore file whose ptah-owned content is a managed ignore section (see below), which SHALL ignore the two package-generated paths anchored to `.ptah/`: `luau_packages/` and `.pesde/`.

The four files have different ownership and different existing-file semantics. `.ptah/config.toml` and `.ptah/pesde.toml` are user-authored: an existing file SHALL be skipped with a per-file skipped message and never modified. `.ptah/ptah.d.luau` is a derived artifact of the installed binary: init SHALL sync it — created when absent, overwritten with the current binary's emitted definitions whenever its bytes differ from the current output in either accepted form (the emitted output itself, or the emitted output without its header line), and left untouched only when it already matches one of those forms byte-for-byte. The headerless arm exists so a file laid out like the repository's source definitions (the headerless compile input) reports as current instead of being rewritten with a prepended header. Overwrite is otherwise unconditional: a differing file with no parseable ptah version header (hand-edited or foreign) SHALL still be overwritten.

`.ptah/.gitignore` is a user-owned file embedding a ptah-owned managed section — a marker-delimited block opened by a line beginning `# >>> ptah` and closed by the line `# <<< ptah`, whose content is the anchored ignore rules for `.ptah/luau_packages/` and `.ptah/.pesde/` plus an explanatory comment naming `ptah init` as the section's maintainer. Init SHALL sync the section as derived content: when the file is absent, init SHALL create it containing exactly the managed section; when the file exists without markers, init SHALL append the section without modifying any existing content; when the file exists with markers, init SHALL rewrite the content between the markers to the current section, leaving everything outside the markers byte-for-byte untouched. When the markers are present and the section already matches, the file SHALL not be written. The section SHALL NOT cover `.ptah/runs/`, which keeps its own enclave ignore. The section content SHALL be identical in every context (init never varies it by project state), and the ignore file SHALL be written unconditionally — init SHALL NOT probe for a git repository, and SHALL NOT offer or consult any flag or manifest key governing the section. Outside these two mechanisms — `ptah init`'s managed section and the run-record writer's `.ptah/runs/` enclave file (per the `run-record` capability) — no ptah command SHALL write or modify any ignore file.

`ptah init` SHALL NOT create any other files (no starter script, no editor configuration), SHALL NOT search parent directories for an existing `.ptah`, SHALL NOT write to the user-level config directory, and SHALL NOT perform package installation: init writes no `pesde.lock`, no `luau_packages/` content, and no root `.luaurc`, and makes no network requests. Running `ptah init` twice with the same binary SHALL change no file: the configs are skipped, the definitions already match, and the managed section is already current. On success the command SHALL print exactly one line per file — `created:` or `skipped (exists):` for the configs; `created:`, `updated:`, or `up to date:` for the definitions; `created:`, `appended:`, `updated:`, or `up to date:` for `.ptah/.gitignore` — followed by next-step hints (editing the registry, pointing luau-lsp at the definitions, installing shell completions, adding packages, the ptah skill) to standard output, and exit 0. An `updated:` definitions line SHALL carry the previous and current version in parentheses when the overwritten file's first line parsed as a ptah version header, and no version suffix otherwise. The hints SHALL print on every run, including runs where no file changed. A failure to write any file (for example an unwritable directory) SHALL print an error to standard error and exit 1.

#### Scenario: Fresh init creates both files
- **WHEN** `ptah init` runs in a directory with no `.ptah`
- **THEN** `.ptah/ptah.d.luau`, `.ptah/config.toml`, `.ptah/pesde.toml`, and `.ptah/.gitignore` exist, the process exits 0, and each created file is announced on standard output

#### Scenario: Fresh init writes the managed ignore section
- **WHEN** `ptah init` runs in a directory with no `.ptah`
- **THEN** `.ptah/.gitignore` contains a section delimited by a line beginning `# >>> ptah` and the closing line `# <<< ptah`, with anchored rules ignoring `luau_packages/` and `.pesde/` relative to `.ptah/` and no rule for `runs/`

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
- **THEN** the configs are reported as skipped (exists), the definitions are reported as up to date, the ignore file is reported as up to date, no file's bytes change, and the process exits 0

#### Scenario: Unmarked ignore file gains the section
- **WHEN** `ptah init` runs where `.ptah/.gitignore` exists with user-written content and no ptah markers
- **THEN** the managed section is appended, every byte that preceded it is unchanged, the operation is announced as appended, and the process exits 0

#### Scenario: Marked section is refreshed
- **WHEN** `ptah init` runs where `.ptah/.gitignore` has ptah markers whose section content differs from the current section (for example written by an older binary)
- **THEN** only the content between the markers changes to the current section, everything outside the markers is unchanged, the operation is announced as updated, and the process exits 0

#### Scenario: User content outside the markers is preserved
- **WHEN** `ptah init` runs where `.ptah/.gitignore` carries user rules both before the opening marker and after the closing marker
- **THEN** those rules are byte-for-byte unchanged and the managed section is current

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
- **THEN** it completes successfully having written only the four scaffold files

#### Scenario: Hints print on every run
- **WHEN** `ptah init` completes, whether files were created, updated, appended, or unchanged
- **THEN** the next-step hints appear on standard output

#### Scenario: Unwritable target fails cleanly
- **WHEN** `ptah init` cannot create `./.ptah` (for example a read-only parent directory)
- **THEN** an error is printed to standard error and the process exits 1
