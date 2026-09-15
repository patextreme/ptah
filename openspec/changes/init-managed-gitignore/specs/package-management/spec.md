## MODIFIED Requirements

### Requirement: Package command exit codes and guidance
Package commands SHALL exit 0 on success, 2 for usage errors (unknown flags, invalid spec forms, unknown remove alias, no project found, unparseable manifest, a malformed default index override (`PTAH_DEFAULT_INDEX`) — reported by the operations that would consult the default index), and 1 for operational failures (resolution conflicts, registry or network failures, missing or stale lockfile under `--locked`, unsupported target). Package commands SHALL never write or modify any ignore file: ptah writes ignore content only inside `.ptah/`, only through `ptah init`'s managed section in `.ptah/.gitignore` (and the `runs/` enclave file), and never the repository's root ignore file. The first successful mutating command in a project SHALL print guidance stating which files to commit (`pesde.toml`, `pesde.lock`, the root `.luaurc`, and `.ptah/.gitignore`) and that the managed section in `.ptah/.gitignore` already ignores the generated directories (`luau_packages/`, `.pesde/`).

#### Scenario: Usage error exits 2
- **WHEN** `ptah package add` runs with no package argument
- **THEN** a usage error is printed and the process exits 2

#### Scenario: Guidance names the files
- **WHEN** `ptah package add` succeeds in a project
- **THEN** standard output includes the guidance naming `pesde.toml`, `pesde.lock`, `.luaurc`, and `.ptah/.gitignore` among the files to commit, and `luau_packages/` and `.pesde/` as the generated directories the managed section ignores

#### Scenario: Package commands never write ignore files
- **WHEN** any `ptah package` command succeeds in a project whose `.ptah/.gitignore` is absent or lacks the managed section
- **THEN** the command completes without creating or modifying any ignore file, and exits without writing one
