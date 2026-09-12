## MODIFIED Requirements

### Requirement: package add
`ptah package add` SHALL accept a bare registry package `scope/name[@version]` (version defaults to the newest compatible release, recorded as a caret requirement), a git source via `--git <url>` with optional `--rev <rev>` and `--path <subdir>`, and a local source via `--path <dir>`. The dependency's alias SHALL default to the package name's last path segment (or the repository name for git sources) and `--as <alias>` SHALL override it; the alias MUST be a valid Pesde alias. After editing the manifest, `add` SHALL run a full install in the same invocation (resolve, link, write lockfile); `--no-install` SHALL limit the command to the manifest edit. A package name that resolves to no versions SHALL produce a diagnostic naming the requested spec and exit 1. When the source is a git repository, resolution SHALL use the branch tip fetched during the current invocation: adding a `--rev <branch>` dependency SHALL succeed even when the repository's cache was previously cloned at an older commit of that branch.

#### Scenario: Add from a git source with a subdirectory
- **WHEN** `ptah package add --git <fixture-git-url> --path pkg/hello` runs in a project
- **THEN** the manifest records a git dependency with the subdirectory, the lockfile pins the resolved commit, and the package is installed

#### Scenario: Add resolves the newest version
- **WHEN** `ptah package add <scope>/<name>` runs against a fixture registry serving versions `0.1.0` and `0.2.0`
- **THEN** the manifest records `^0.2.0` and the lockfile pins `0.2.0`

#### Scenario: Unknown package
- **WHEN** `ptah package add <scope>/does-not-exist` runs against the fixture registry
- **THEN** a diagnostic names the package spec and the process exits 1

#### Scenario: Add a branch rev against a warm cache
- **WHEN** a git repository's branch advanced after an earlier ptah package operation cached an older commit of that branch, and `ptah package add --git <fixture-git-url> --rev <branch>` runs
- **THEN** the dependency resolves at the branch's current tip, the lockfile pins that commit, and the package is installed

### Requirement: package install
`ptah package install` SHALL resolve the manifest against the registry, download, and link packages into `luau_packages/`, and write `pesde.lock` when the graph changed. An install with an up-to-date lockfile and populated packages SHALL change no file and exit 0. Network, registry, and dependency-conflict failures SHALL produce diagnostics naming the failing step or package and exit 1. An install that reuses a compatible lockfile SHALL keep each dependency's pinned version — including a git dependency recorded with a branch rev or `HEAD` — and SHALL NOT advance it; only `package update` advances such dependencies.

#### Scenario: Install is idempotent
- **WHEN** `ptah package install` runs twice in a row in a satisfied project
- **THEN** the second run changes no file (manifest, lockfile, and linker files byte-identical) and exits 0

#### Scenario: Registry failure
- **WHEN** `ptah package install` runs and the fixture registry rejects the request
- **THEN** a diagnostic names the registry failure and the process exits 1

#### Scenario: Install keeps a git branch dependency pinned
- **WHEN** the lockfile pins a git dependency at commit A, the repository's branch has advanced to commit B, and `ptah package install` runs
- **THEN** the lockfile still records commit A and the installed files match commit A

### Requirement: package update
`ptah package update` SHALL re-resolve every dependency from scratch (ignoring the previous lockfile for version selection), install the result, and write the new lockfile. It takes no package argument. Re-resolving a git dependency whose recorded rev is a branch name or `HEAD` SHALL resolve against the branch tip fetched during the current invocation, advancing it to the branch's newer commits; a git dependency pinned to a commit SHA SHALL stay at that commit.

#### Scenario: Update moves a caret within range
- **WHEN** the manifest pins `^0.1.0`, the lockfile holds `0.1.0`, the fixture registry later serves `0.1.1`, and `ptah package update` runs
- **THEN** the lockfile records `0.1.1` and the installed files match it

#### Scenario: Update advances an unpinned git dependency
- **WHEN** a git dependency is recorded with rev `HEAD`, the lockfile pins the branch's earlier commit A, the repository's branch has since advanced to commit B, and `ptah package update` runs
- **THEN** the lockfile records commit B and the installed files match commit B

#### Scenario: Update keeps a commit-pinned git dependency
- **WHEN** a git dependency is recorded with an explicit commit SHA and the repository's branch has advanced, and `ptah package update` runs
- **THEN** the lockfile still records that commit SHA and the installed files match it
