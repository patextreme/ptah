# Script Checking Capability Delta

## MODIFIED Requirements

### Requirement: Static lints walk the literal require graph
The check SHALL statically analyze the entry and every file reachable through literal string `require("...")` call arguments, resolving each path relative to its requiring file under ptah's module-resolution rules (no boundary: requires may traverse out of the entry script's directory), without executing anything. It SHALL report:

- **Unknown agent names**: a literal `ptah.agent("<name>")` string argument that resolves in no discovered registry (project `.ptah/config.toml` found upward from the invocation directory overriding the user config per agent name, exactly as `run` discovers) is a finding. Non-literal (computed) arguments and inline spec tables SHALL NOT be linted.
- **Broken requires**: a literal require target that does not resolve to an existing module file (`.luau`, `.lua`, `init.luau`, `init.lua`) is a finding. A require whose target exists outside the entry script's directory is NOT a finding.
- **Missing strict directive**: the entry and every reachable file SHALL begin with a `--!strict` directive; a file without it is a finding.
- **Unusable interaction**: `ptah.ask(` member calls on the `ptah` global (any argument form; the call site itself is the signal, unlike agent names) are collected over the reachable set. With at least one ask call site present, the check SHALL resolve the interaction provider under the same selection chain `run` uses (`--ask` flag > `PTAH_ASK` > project `[ask]` > user `[ask]` > automatic detection) and SHALL report a finding when the resolution is `none` (prohibited) or when nothing resolved explicitly and automatic detection cannot apply (stdin or stdout is not a terminal — the CI case), the finding naming the remedies (`--ask`, `PTAH_ASK`, `[ask]`). With no ask call sites, the resolved posture SHALL NOT produce a finding. Alias-indirected asks (e.g. `local f = ptah.ask`) are not collected — a documented residual; the runtime check remains the source of truth.

#### Scenario: Unknown literal agent name
- **WHEN** a reachable file contains `ptah.agent("clawed")` and no registry defines `clawed`
- **THEN** the check reports a finding naming the agent and exits 1

#### Scenario: Computed agent name is not linted
- **WHEN** a reachable file contains `ptah.agent(name)` where `name` is a variable
- **THEN** the check reports no finding for that call

#### Scenario: Require outside the entry tree is not a finding
- **WHEN** a reachable file contains `require("../../outside")` and the target resolves to an existing module file outside the entry script's directory
- **THEN** the check reports no finding for that require

#### Scenario: Missing module
- **WHEN** a reachable file contains `require("./lib/nope")` and no such module file exists
- **THEN** the check reports a finding naming the unresolved path

#### Scenario: Missing strict directive in a module
- **WHEN** the entry declares `--!strict` but a reachable required module does not
- **THEN** the check reports a finding naming the module file and exits 1

#### Scenario: Registry agent resolves
- **WHEN** a reachable file contains `ptah.agent("claude")` and any discovered registry defines `claude`
- **THEN** the check reports no finding for that call

#### Scenario: Ask with prohibited posture is a finding
- **WHEN** a reachable file contains `ptah.ask({ prompt = "q" })` and the resolved provider is `none` (e.g. `--ask=none` or `[ask] provider = "none"`)
- **THEN** the check reports a prohibited-interaction finding and exits 1

#### Scenario: Ask unresolvable off-terminal is a finding
- **WHEN** a reachable file contains `ptah.ask(...)` and the check runs with no `--ask`, `PTAH_ASK`, or `[ask]` on a non-interactive stdin or stdout
- **THEN** the check reports a finding naming `--ask`, `PTAH_ASK`, and `[ask]` as remedies, and exits 1

#### Scenario: Ask resolves explicitly in CI
- **WHEN** the same check runs non-interactively with `PTAH_ASK=stdin`
- **THEN** the interaction resolution is clean and no interaction finding is reported

#### Scenario: No ask call sites means no interaction finding
- **WHEN** a reachable set contains no `ptah.ask` calls and the resolved provider is `none`
- **THEN** the check reports no interaction finding

#### Scenario: Aliased ask is not collected
- **WHEN** a reachable file contains `local f = ptah.ask` and calls `f(...)` with the resolved provider `none`
- **THEN** the check reports no interaction finding (documented residual); running the script raises the prohibited error at call time
