# Package Management Specification

## Purpose

First-class package management for ptah projects: installing, versioning, and updating workflow packages (Factory Components material) through `ptah package` commands, backed by an embedded Pesde engine so no separate package-manager binary is required.

## Requirements

### Requirement: Package command group and project discovery
The CLI SHALL provide a `ptah package` command group with the subcommands `add`, `remove`, `install`, and `update`. All package commands SHALL operate on the package project rooted at the nearest `.ptah` directory found by walking upward from the invocation directory for a directory containing `config.toml` or `pesde.toml`, so the commands work from any subdirectory inside a project. Package commands SHALL NOT search upward past a filesystem root, and when no project is found SHALL print an error naming what was searched for and exit 2.

#### Scenario: Run from a nested directory
- **WHEN** `ptah package install` runs in `<project>/.ptah/workflows/openspec/` and `<project>/.ptah/pesde.toml` exists
- **THEN** the command operates on `<project>/.ptah` and succeeds

#### Scenario: No project found
- **WHEN** `ptah package install` runs in a directory with no `.ptah` project above it
- **THEN** an error is printed to standard error and the process exits 2

### Requirement: Package project layout
The package project SHALL live inside `.ptah/` using Pesde's conventional names: the manifest at `.ptah/pesde.toml`, the lockfile at `.ptah/pesde.lock`, installed packages under `.ptah/luau_packages/`, and the package cache under `.ptah/.pesde/`. `pesde.toml` and `pesde.lock` are user-owned artifacts meant to be committed; `luau_packages/` and `.pesde/` are generated and meant to be ignored by source control. ptah SHALL treat the manifest target as fixed: package commands SHALL require the manifest's target environment to be `luau` and SHALL report a clear diagnostic and exit 1 for any other target.

#### Scenario: Install populates the conventional layout
- **WHEN** `ptah package add <fixture-package>` succeeds in a project
- **THEN** `.ptah/pesde.toml` records the dependency, `.ptah/pesde.lock` exists, and the package is linked under `.ptah/luau_packages/`

#### Scenario: Non-luau target is rejected
- **WHEN** a manifest declares `environment = "roblox"`
- **THEN** the package command prints a diagnostic naming the unsupported target and exits 1

### Requirement: Manifest scaffolding and ownership
`ptah init` SHALL create `.ptah/pesde.toml` when absent, with a skeleton that parses as a valid Pesde manifest: a `private` package named `components/<lowercased-directory-name>`, target environment `luau`, no dependency entries, and no `[indices]` table. An existing `pesde.toml` SHALL be skipped with a per-file skipped message and never modified by init. Package commands edit the manifest surgically (dependency tables only), preserving all other manifest content including user-added `[indices]` entries.

#### Scenario: Skeleton parses as a valid manifest
- **WHEN** the `pesde.toml` written by `ptah init` is parsed as a Pesde manifest
- **THEN** it parses without error, is private, targets the `luau` environment, and contains no dependencies or indices

#### Scenario: User manifest content is preserved
- **WHEN** `ptah package add` writes a dependency into a manifest that also contains a user-authored `[indices]` table
- **THEN** the dependency is added and the `[indices]` table is byte-identical afterwards

### Requirement: Default registry index
When the manifest has no `[indices]` table, package commands SHALL resolve bare package names against a default registry index URL supplied internally by ptah; the skeleton and manifest edits SHALL NOT write an `[indices]` table. A user-added `[indices]` table SHALL take precedence exactly as Pesde natively defines, and package commands SHALL respect it without ptah-level registry configuration.

#### Scenario: Bare name installs from the default index
- **WHEN** `ptah package add <scope>/<name>` runs in a project whose manifest has no `[indices]` and the fixture registry serves `<scope>/<name>`
- **THEN** the package resolves, installs, and no `[indices]` table appears in the manifest

#### Scenario: Custom index is respected
- **WHEN** the manifest declares `[indices] default = "<fixture-index>"` and `ptah package add <scope>/<name>` runs
- **THEN** resolution uses the declared index, not ptah's internal default

### Requirement: package add
`ptah package add` SHALL accept a bare registry package `scope/name[@version]` (version defaults to the newest compatible release, recorded as a caret requirement), a git source via `--git <url>` with optional `--rev <rev>` and `--path <subdir>`, and a local source via `--path <dir>`. The dependency's alias SHALL default to the package name's last path segment (or the repository name for git sources) and `--as <alias>` SHALL override it; the alias MUST be a valid Pesde alias. After editing the manifest, `add` SHALL run a full install in the same invocation (resolve, link, write lockfile); `--no-install` SHALL limit the command to the manifest edit. A package name that resolves to no versions SHALL produce a diagnostic naming the requested spec and exit 1.

#### Scenario: Add from a git source with a subdirectory
- **WHEN** `ptah package add --git <fixture-git-url> --path pkg/hello` runs in a project
- **THEN** the manifest records a git dependency with the subdirectory, the lockfile pins the resolved commit, and the package is installed

#### Scenario: Add resolves the newest version
- **WHEN** `ptah package add <scope>/<name>` runs against a fixture registry serving versions `0.1.0` and `0.2.0`
- **THEN** the manifest records `^0.2.0` and the lockfile pins `0.2.0`

#### Scenario: Unknown package
- **WHEN** `ptah package add <scope>/does-not-exist` runs against the fixture registry
- **THEN** a diagnostic names the package spec and the process exits 1

### Requirement: package remove
`ptah package remove <alias>` SHALL remove the dependency entry with that alias from the manifest and run a full install in the same invocation (or manifest-only with `--no-install`). Removing an alias that is not in the manifest SHALL be a usage error (exit 2).

#### Scenario: Remove reinstalls
- **WHEN** `ptah package remove hello` runs in a project where `hello` is installed
- **THEN** the manifest entry and lockfile entry are gone and the package's files no longer resolve under `luau_packages/`

#### Scenario: Remove unknown alias
- **WHEN** `ptah package remove nope` runs in a project with no such dependency
- **THEN** a usage error is printed and the process exits 2

### Requirement: package install
`ptah package install` SHALL resolve the manifest against the registry, download, and link packages into `luau_packages/`, and write `pesde.lock` when the graph changed. An install with an up-to-date lockfile and populated packages SHALL change no file and exit 0. Network, registry, and dependency-conflict failures SHALL produce diagnostics naming the failing step or package and exit 1.

#### Scenario: Install is idempotent
- **WHEN** `ptah package install` runs twice in a row in a satisfied project
- **THEN** the second run changes no file (manifest, lockfile, and linker files byte-identical) and exits 0

#### Scenario: Registry failure
- **WHEN** `ptah package install` runs and the fixture registry rejects the request
- **THEN** a diagnostic names the registry failure and the process exits 1

### Requirement: Locked install
`ptah package install --locked` SHALL install exactly the versions recorded in `pesde.lock` without re-resolving the manifest against the registry. It SHALL fail with a diagnostic and exit 1 when the lockfile is missing, or when it is stale: the manifest's name, target, overrides, or dependency specifications do not match the lockfile's direct entries.

#### Scenario: Locked install from a fresh clone
- **WHEN** a project with a committed `pesde.lock` has `luau_packages/` removed and `ptah package install --locked` runs
- **THEN** the recorded versions are installed, the lockfile is not rewritten, and the process exits 0

#### Scenario: Missing lockfile
- **WHEN** `ptah package install --locked` runs with no `pesde.lock`
- **THEN** a diagnostic states the lockfile is missing and the process exits 1

#### Scenario: Stale lockfile
- **WHEN** a dependency is added to the manifest without reinstalling and `ptah package install --locked` runs
- **THEN** a diagnostic states the lockfile is out of sync and the process exits 1

### Requirement: package update
`ptah package update` SHALL re-resolve every dependency from scratch (ignoring the previous lockfile for version selection), install the result, and write the new lockfile. It takes no package argument.

#### Scenario: Update moves a caret within range
- **WHEN** the manifest pins `^0.1.0`, the lockfile holds `0.1.0`, the fixture registry later serves `0.1.1`, and `ptah package update` runs
- **THEN** the lockfile records `0.1.1` and the installed files match it

### Requirement: Package alias synchronization
Package commands that change the installed dependency set (`add`, `remove`, `install`, `update`) SHALL synchronize the package aliases into a `.luaurc` file at the project root (the directory containing `.ptah/`): each dependency alias maps to its `luau_packages` entry, in standard Luau `.luaurc` format. ptah SHALL create the file when absent, preserve every other key in the file (both outside and inside `aliases`), preserve alias entries it does not own (those not pointing into the project's `luau_packages/`), and remove or update entries for aliases it owns. A user-owned alias colliding with a package alias SHALL produce a diagnostic and leave the user's entry untouched.

#### Scenario: Add syncs the root luaurc
- **WHEN** `ptah package add <fixture-package> --as hello` succeeds in a project without a root `.luaurc`
- **THEN** the project root contains a `.luaurc` whose `aliases` map `hello` to the `luau_packages/hello` entry

#### Scenario: User keys are preserved
- **WHEN** the root `.luaurc` contains a user-authored key (e.g. `languageMode`) and `ptah package install` re-syncs aliases
- **THEN** the user key and its value are byte-preserved and only ptah-owned alias entries change

### Requirement: Package command exit codes and guidance
Package commands SHALL exit 0 on success, 2 for usage errors (unknown flags, invalid spec forms, unknown remove alias, no project found, unparseable manifest), and 1 for operational failures (resolution conflicts, registry or network failures, missing or stale lockfile under `--locked`, unsupported target). Package commands SHALL never edit version-control ignore files; the first successful mutating command in a project SHALL print guidance stating which files to commit (`pesde.toml`, `pesde.lock`, the root `.luaurc`) and which directories to ignore (`luau_packages/`, `.pesde/`).

#### Scenario: Usage error exits 2
- **WHEN** `ptah package add` runs with no package argument
- **THEN** a usage error is printed and the process exits 2

#### Scenario: Guidance names the files
- **WHEN** `ptah package add` succeeds in a project
- **THEN** standard output includes the commit/ignore guidance naming `pesde.toml`, `pesde.lock`, `.luaurc`, `luau_packages/`, and `.pesde/`
