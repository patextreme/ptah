# Factory Components Specification

## Purpose

The shared workflow library (`factory-components/`): repo-agnostic helper
modules and composable workflow components that consumer repositories mount
as source and drive through thin shims, replacing per-repo copies of the
same agent-orchestration machinery.

## Requirements

### Requirement: Typed judge

The library SHALL provide a typed boolean judge that asks a designated agent
model whether a predicate holds for a payload and returns the verdict from
the session's typed boolean result. A judge that submits no verdict SHALL be
retried a bounded number of times, and exhaustion SHALL be a script error,
never a hang or a silent default.

#### Scenario: Verdict returned

- **WHEN** the judge session submits a boolean verdict for the predicate and payload
- **THEN** the operation returns that verdict

#### Scenario: Judge never answers

- **WHEN** the judge session submits no typed verdict on every attempt up to the bound
- **THEN** the operation raises a script error naming the judge and the attempt count

### Requirement: GitHub CLI transport

The library SHALL provide a GitHub transport that shells out to the `gh` CLI
via ptah's exec, returning a structured outcome (exit code, stdout, parsed
JSON when requested) instead of raising, and SHALL quote arguments so values
containing spaces or quotes are passed through verbatim.

#### Scenario: Command succeeds with JSON output

- **WHEN** a `gh` invocation exits zero and JSON output is requested
- **THEN** the transport returns a success outcome carrying the parsed JSON value

#### Scenario: Command fails

- **WHEN** a `gh` invocation exits non-zero
- **THEN** the transport returns a failure outcome carrying the exit code and stderr, and the calling script keeps running

### Requirement: Daemon loop skeleton

The library SHALL provide a repo-loop skeleton that applies a per-repo
operation to every configured repository, isolates each repository behind an
error boundary so one raising repository cannot abort the others, and
supports both sequential and bounded-concurrency parallel execution.

#### Scenario: One repository raises

- **WHEN** the per-repo operation raises for one of several configured repositories
- **THEN** the loop records that repository's failure and completes the remaining repositories

### Requirement: Component facade contract

Every workflow component SHALL be a module exposing a constructor that
accepts a data config table and returns an instance whose methods are the
component's typed operations; config SHALL NOT contain callable hooks.
Ptah runtime handles (e.g. agent handles) SHALL be permitted as config
values where the component's exported config type declares them.
Components and stdlib modules SHALL be strict-mode typed so that `ptah check`
in a consumer repo validates the consumer's config against the component's
config type.

#### Scenario: Consumer config mismatches the component type

- **WHEN** a consumer shim passes a config table that does not satisfy the component's exported config type and runs `ptah check`
- **THEN** check reports a type error naming the offending field

#### Scenario: Per-call data is a method argument

- **WHEN** a consumer calls a component operation that acts on a specific item (e.g. a change name or PR URL)
- **THEN** the item is supplied as a method argument, not baked into the component's config

#### Scenario: Agent handle accepted as config

- **WHEN** a consumer constructs an agent handle (from a registry name or an inline agent spec) and passes it in a config field the component's config type declares as an agent handle
- **THEN** the component drives that role's sessions through the supplied handle and performs no agent construction of its own

#### Scenario: Callable hook rejected by the type gate

- **WHEN** a consumer shim passes a config containing an arbitrary function in a config field and runs `ptah check`
- **THEN** check reports a type error naming the offending field

### Requirement: Library self-containment

Library modules SHALL only require other modules within the library tree, so
the tree works mounted at any path; they SHALL NOT write files inside the
library tree; and they SHALL NOT invoke exec with a relative working
directory — repository-relative paths MUST arrive through config.

#### Scenario: Mounted at an arbitrary path

- **WHEN** the library tree is mounted at any directory inside a consumer repo and a shim requires a component by relative path
- **THEN** the component and its internal requires resolve without reference to the mount location

#### Scenario: Library tree is read-only

- **WHEN** a component runs from a read-only mount (e.g. the nix store)
- **THEN** the workflow completes without attempting to write inside the library tree

### Requirement: openspec component

The library SHALL provide an openspec component whose instance exposes
groom, implement, and verify operations on a named change: groom converges a
change's proposals through review, implement drives task execution, and
verify converges verification then syncs and archives the change. The
component SHALL declare its environment requirements (an agent carrying the
openspec skills, `openspec` on PATH) in its documentation rather than
bundling or installing them. Convergence is the component's own loop over
the library's typed judge: a judge-rejected pass probes for human input,
a needed human fails the operation without issuing a fix, and exhausting
the iteration cap fails the operation with an error reporting the cap.

#### Scenario: Verify converges and archives

- **WHEN** verify runs on a change whose implementation passes the verification judge
- **THEN** the verification loop exits and the sync-and-archive step runs as part of the same operation

#### Scenario: Missing environment requirement

- **WHEN** the component's documentation is consulted for its environment requirements
- **THEN** the agent-skill and CLI requirements are listed so a consumer can verify them before running

#### Scenario: Human escalation

- **WHEN** a groom pass is judge-rejected and the escalation judge confirms human input is required
- **THEN** the operation fails with an error stating human input is needed, and no fix prompt is issued

#### Scenario: Iteration cap

- **WHEN** every pass is judge-rejected and the findings stay fixable up to the configured iteration cap
- **THEN** the operation fails with an error reporting the cap was reached

### Requirement: PR review loop component

The library SHALL provide a PR review loop component that runs a
review→fix→push convergence against a pull request, with repository-specific
settings (agent prompts, reviewer instruction text, dry-run gating)
expressed as config rather than code. The target repository SHALL NOT be
component config: it arrives per call inside the PR URL.

#### Scenario: Review finds fixable findings

- **WHEN** the reviewer reports findings judged resolvable without a human
- **THEN** the component drives a fix session and pushes, iterating until the review passes or escalation occurs

#### Scenario: Repository context is per-call

- **WHEN** the loop reviews a pull request
- **THEN** the repository context comes from the PR URL passed to the operation, and the component's config declares no repository field

### Requirement: PR review instruction contract

The pr-review-loop component's documentation SHALL declare the contract its
reviewer instruction must satisfy: a configured reviewer instruction defines
what counts as a blocking issue for the repository and instructs the
reviewer to classify findings as blocking or non-blocking, and the
component's judge predicates and fix prompts speak that classification
vocabulary. The component's config surface (the exported `Config` type's
doc comment for `reviewInstruction`) SHALL state the classification
requirement. The documentation SHALL also state the component's boundary:
verdicts that do not reduce to a blocking/non-blocking classification
(score gates, approve/request-changes, report-only reviews) are a different
component, not an instruction swap.

The documentation SHALL present pointer-style instructions — reviewer
instruction text that references a repository document — as the recommended
form when the instruction is long or repo-pinned, noting that configured
text is inlined into every iteration's review prompt.

The component SHALL ship a built-in default instruction that satisfies this
contract and SHALL use it when no reviewer instruction is configured; a
configured reviewer instruction SHALL take precedence over the built-in
default, as a full replacement (the configured text is the entire
instruction, inlined into the review prompt). Only a nil `reviewInstruction`
selects the built-in default.

#### Scenario: Instruction contract is declared

- **WHEN** a consumer consults the pr-review-loop component's documentation before supplying a reviewer instruction
- **THEN** the required blocking/non-blocking verdict classification is stated, along with the boundary that verdicts not reducible to it belong to a different component

#### Scenario: Config surface states the classification requirement

- **WHEN** a consumer reads the exported `Config` type for the pr-review-loop component
- **THEN** the `reviewInstruction` field's documentation states that a configured reviewer instruction must classify findings as blocking or non-blocking, and that a nil value selects the built-in default

#### Scenario: Built-in default instruction used when none is configured

- **WHEN** the component is configured without `reviewInstruction` (the field is nil)
- **THEN** reviews run against the component's built-in default instruction, which directs the reviewer to classify each finding as blocking or non-blocking (the default is the contract's reference instance)

#### Scenario: Configured instruction takes precedence

- **WHEN** `reviewInstruction` is configured with instruction text
- **THEN** the built-in default is not used — the configured text is inlined into the review prompt, the default's classification directive is absent, and the configured instruction governs the review

#### Scenario: Pointer pattern is the documented long form

- **WHEN** a consumer consults the pr-review-loop component's documentation with a long or repo-pinned reviewer instruction in mind
- **THEN** the documentation presents pointer-style text referencing a repository document as the recommended form

### Requirement: Dogfooded in this repository

This repository's own `.ptah/workflows/*` SHALL be shims that require the
library rather than carrying their own helper copies; after this change no
workflow helper module SHALL be duplicated under `.ptah/`.

#### Scenario: Repo workflow uses the library

- **WHEN** the openspec groom and verify workflows in this repository run
- **THEN** their convergence and judging behavior comes from the library, and grepping `.ptah/` finds no second copy of the judge or transport

### Requirement: Offline test coverage

Every stdlib module and component entry point SHALL be exercised by the
offline test suite against the mock agent, with no network access and no
real agent.

#### Scenario: Library regressions caught offline

- **WHEN** a library module's behavior breaks (e.g. the judge stops returning verdicts)
- **THEN** the offline suite fails without spawning any real agent
