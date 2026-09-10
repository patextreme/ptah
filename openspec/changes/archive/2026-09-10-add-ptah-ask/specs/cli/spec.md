# CLI Capability Delta

## ADDED Requirements

### Requirement: Ask provider override
`ptah run` and `ptah check` SHALL accept `--ask=<provider>`, where `<provider>` is a known provider name (`stdin` or `none` in v1). An unknown value SHALL be a usage error exiting 2. The CLI SHALL honor the `PTAH_ASK` environment variable with the same value set; the flag, when present, takes precedence over the environment. These overrides sit atop the selection chain specified by the ask capability (flag > environment > project `[ask]` > user `[ask]` > automatic detection), and the resolved provider is fixed for the whole run.

#### Scenario: Flag rejects an unknown provider
- **WHEN** `ptah run --ask=webhook script.luau` is invoked (webhook not a v1 provider)
- **THEN** the CLI prints a usage error naming the accepted values and exits 2

#### Scenario: Environment applies without the flag
- **WHEN** `ptah run script.luau` runs with `PTAH_ASK=none`
- **THEN** interaction is prohibited for the run

#### Scenario: Flag beats environment
- **WHEN** `ptah run --ask=stdin script.luau` runs with `PTAH_ASK=none`
- **THEN** the stdin provider is active

#### Scenario: Check accepts the flag
- **WHEN** `ptah check --ask=stdin script.luau` runs on a script that calls `ptah.ask`
- **THEN** the check's interaction resolution sees the stdin provider (no unresolvable-provider finding)

## MODIFIED Requirements

### Requirement: Run pre-flight fails certain-broken scripts before spawning
`ptah run` SHALL perform an in-process pre-flight before executing the script: compile/parse the entry and every file reachable through literal `require("...")` string arguments, resolve literal require targets under ptah's module-resolution rules (existence; no boundary — requires may traverse out of the entry script's directory), resolve literal `ptah.agent("<name>")` string arguments against the discovered registry, and resolve `ptah.ask(` member calls on the `ptah` global (any argument form) over the same reachable set. A pre-flight failure SHALL fail the run before any agent subprocess spawns, with the finding(s) printed to standard error and exit code 1.

The pre-flight SHALL NOT execute script code, SHALL NOT enforce the `--!strict` directive, and SHALL NOT invoke `luau-lsp`. Non-literal (computed) require paths and agent names SHALL NOT be pre-flighted — a script using them runs exactly as before.

For ask call sites, pre-flight SHALL resolve the interaction provider under the full selection chain (`--ask` > `PTAH_ASK` > project `[ask]` > user `[ask]` > automatic detection) and SHALL fail when: the resolved provider is `none` (prohibited — a deliberate posture contradicting the script's need), or no explicit selection resolved and automatic detection cannot (neither stdin nor stdout is a terminal), with a finding naming the remedies (`--ask`, `PTAH_ASK`, `[ask]`). A script with no ask call sites SHALL NOT be failed by interaction posture regardless of the resolved provider. The pre-flight finding is over-approximate by design (an ask on a never-executed branch still fails under `none`) and it does not replace the runtime check.

#### Scenario: Unknown literal agent name fails fast
- **WHEN** `ptah run script.luau` runs and the script contains `ptah.agent("clawed")` where no registry defines `clawed`
- **THEN** the run fails before any agent subprocess spawns and exits 1

#### Scenario: Broken literal require fails fast
- **WHEN** a script contains `require("./lib/missing")` and no such module file exists
- **THEN** the run fails immediately with a finding naming the unresolved path, before any agent spawns

#### Scenario: Cross-tree require passes pre-flight
- **WHEN** a script contains `require("../shared/util")` and the module exists outside the entry script's directory
- **THEN** the pre-flight resolves it without findings and the run proceeds

#### Scenario: Non-strict scripts still run
- **WHEN** a script without a `--!strict` directive is run
- **THEN** the run proceeds exactly as before; the directive is not required for execution

#### Scenario: Computed agent name is not pre-flighted
- **WHEN** a script calls `ptah.agent(name)` with a variable
- **THEN** the pre-flight makes no claim about it and the run proceeds

#### Scenario: Unreachable missing require is accepted risk
- **WHEN** a script requires a missing module on a code path that never executes at runtime
- **THEN** the pre-flight still fails the run (documented, accepted false-positive class)

#### Scenario: Ask with prohibited posture fails fast
- **WHEN** a script contains `ptah.ask({ prompt = "q" })` and the run resolves to `--ask=none`
- **THEN** the run fails before any agent subprocess spawns with a prohibited finding, and exits 1

#### Scenario: Ask unresolvable without a terminal fails fast
- **WHEN** a script contains `ptah.ask({ prompt = "q" })`, the run has no `--ask`, `PTAH_ASK`, or `[ask]`, and stdin or stdout is not a terminal
- **THEN** the run fails with a finding naming the selection remedies, and exits 1

#### Scenario: Ask with explicit provider passes without a terminal
- **WHEN** the same script runs non-interactively with `PTAH_ASK=stdin`
- **THEN** the pre-flight resolves the stdin provider and the run proceeds

#### Scenario: No ask call sites ignores the posture
- **WHEN** a script never calls `ptah.ask` and the run resolves to `none`
- **THEN** the pre-flight reports no interaction finding and the run proceeds
