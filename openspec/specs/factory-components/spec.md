# Factory Components Specification

## Purpose

The shared workflow library (`factory-components/`): repo-agnostic helper
modules and composable workflow components that consumer repositories mount
as source and drive through thin shims, replacing per-repo copies of the
same agent-orchestration machinery.

## Requirements

### Requirement: Session config application

The library SHALL provide a shared session-config mechanism: an ordered
array of entries, each carrying an option `id` and a `value` of string or
boolean, applied to a session as `setConfig` calls in declared order after
session creation and before the session's first prompt. The array is the
consumer's `setConfig` call sequence expressed as data. The mechanism SHALL
export the entry type and the apply operation so consumers building their
own components use it verbatim, and the library's own components and judge
SHALL use it so application semantics cannot drift.

A nil or empty entry array SHALL apply nothing. Duplicate ids SHALL apply
verbatim in order (the later entry wins, exactly as repeated runtime
`setConfig` calls would). An agent-rejected entry SHALL fail through the
existing `setConfig` error path — a catchable error carrying the option id
and the agent's message — and entries applied before the rejection SHALL
remain applied.

#### Scenario: Entries apply in declared order

- **WHEN** a session is configured with entries `{ id = "model", value = "opus" }` followed by `{ id = "effort", value = "high" }`
- **THEN** the session receives the two `setConfig` calls in exactly that order before its first prompt

#### Scenario: Omitted or empty entry array is a no-op

- **WHEN** a component or judge is configured without session-config entries, or with an empty array
- **THEN** no `setConfig` calls are issued and the session's behavior is otherwise unchanged

#### Scenario: Duplicate ids apply verbatim

- **WHEN** the configured entries contain the same id twice with different values
- **THEN** both apply in declared order and the later value is the effective one, matching repeated runtime `setConfig` calls

#### Scenario: Agent rejection fails through the existing error path

- **WHEN** the agent rejects a configured entry's `setConfig`
- **THEN** the operation raises the `setConfig` error carrying the option id and the agent's message, and entries applied before the rejection remain applied

#### Scenario: Malformed entries are a check finding

- **WHEN** a consumer shim declares a session-config entry with a wrong-typed value or a missing id and runs `ptah check`
- **THEN** check reports a type error naming the entry shape

### Requirement: Typed judge

The library SHALL provide a typed boolean judge that asks a designated
judge agent whether a predicate holds for a payload and returns the verdict
from the session's typed boolean result. The judge's options SHALL accept
an agent handle, a session id, a retry bound, and an optional ordered array
of session-config entries applied to every judge attempt session before
its prompt; no dedicated model field SHALL exist (a model choice is an
ordinary session-config entry). A judge that submits no verdict SHALL be
retried a bounded number of times, and exhaustion SHALL be a script error,
never a hang or a silent default.

#### Scenario: Verdict returned

- **WHEN** the judge session submits a boolean verdict for the predicate and payload
- **THEN** the operation returns that verdict

#### Scenario: Judge never answers

- **WHEN** the judge session submits no typed verdict on every attempt up to the bound
- **THEN** the operation raises a script error naming the judge and the attempt count

#### Scenario: Judge sessions receive configured entries

- **WHEN** the judge is invoked with session-config entries
- **THEN** every attempt session receives the entries in declared order before the predicate prompt

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
Data config SHALL include arrays of typed data records (e.g. ordered
session-config entries) wherever the component's exported config type
declares them. Ptah runtime handles (e.g. agent handles) SHALL be permitted
as config values where the component's exported config type declares them.
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

#### Scenario: Ordered entries are data config

- **WHEN** a consumer configures a component's declared session-config field with an ordered array of entry records
- **THEN** the configuration is accepted as data config and validated against the field's declared entry type

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

The component's config SHALL accept `sessionConfig`, an ordered
session-config entry array applied to every work session the component
creates — every per-iteration session of groom, implement, and verify, and
verify's archive session — and `judgeSessionConfig`, applied to every judge
and human-escalation-probe session. The `model` and `judgeModel` config
fields SHALL NOT exist: a model choice is an ordinary `sessionConfig`
entry, and the entry order is the consumer's `setConfig` order.

implement SHALL accept an optional task scope: free text describing the
subset of the change's tasks the run is responsible for. When a scope is
given, the work prompt SHALL instruct the agent to treat the scoped tasks
as the entire job and leave all other tasks pending, and the loop's
acceptance SHALL be judged against the scope rather than against the
whole change. The scope SHALL be carried to the judge so the verdict is
made against the scoped completion, without the judge needing the work
prompt. A scope that matches no tasks SHALL end the pass with a stated
dead-end rather than the agent substituting a different subset, and the
existing human-escalation path SHALL surface it as an operation error.
groom and verify SHALL remain whole-change operations.

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

#### Scenario: Scoped implement completes the subset

- **WHEN** implement runs with a task scope and the agent implements the tasks matching that scope
- **THEN** the judge accepts the pass and the operation exits successfully while tasks outside the scope remain pending

#### Scenario: Scopeless implement is unchanged

- **WHEN** implement runs without a task scope
- **THEN** the prompts, judge acceptance, and log lines are identical to the behavior before task scopes existed, with completion judged against all tasks of the change

#### Scenario: Unresolvable scope dead-ends

- **WHEN** implement runs with a task scope that matches no tasks of the change
- **THEN** the agent ends the pass stating that the scope matches no tasks without implementing a substitute subset, and the operation fails through the human-escalation path

#### Scenario: Work sessions receive session config

- **WHEN** the component is configured with `sessionConfig` entries and any operation runs
- **THEN** every per-iteration work session, and verify's archive session, receives the entries in declared order before its first prompt

#### Scenario: Judge and probe sessions receive judge session config

- **WHEN** the component is configured with `judgeSessionConfig` entries and an operation runs
- **THEN** every judge session and every human-escalation-probe session receives the entries in declared order before its prompt

#### Scenario: Removed model field is a type error

- **WHEN** a consumer shim configures the removed `model` or `judgeModel` field and runs `ptah check`
- **THEN** check reports a type error naming the unknown field, steering the consumer to the `sessionConfig` entry form

### Requirement: PR review loop component

The library SHALL provide a PR review loop component that runs a
review→fix→push convergence against a pull request, with repository-specific
settings (agent prompts, reviewer instruction text, dry-run gating)
expressed as config rather than code. The target repository SHALL NOT be
component config: it arrives per call inside the PR URL.

The component's config SHALL accept `sessionConfig`, an ordered
session-config entry array applied to every work session the component
creates (each review/fix iteration's session, which also posts the verdict
comment), and `judgeSessionConfig`, applied to every judge and
human-escalation-probe session. The `model` and `judgeModel` config
fields SHALL NOT exist: a model choice is an ordinary `sessionConfig`
entry, and the entry order is the consumer's `setConfig` order.

#### Scenario: Review finds fixable findings

- **WHEN** the reviewer reports findings judged resolvable without a human
- **THEN** the component drives a fix session and pushes, iterating until the review passes or escalation occurs

#### Scenario: Repository context is per-call

- **WHEN** the loop reviews a pull request
- **THEN** the repository context comes from the PR URL passed to the operation, and the component's config declares no repository field

#### Scenario: Work sessions receive session config

- **WHEN** the component is configured with `sessionConfig` entries and the review loop runs
- **THEN** every per-iteration work session receives the entries in declared order before its first prompt

#### Scenario: Judge and probe sessions receive judge session config

- **WHEN** the component is configured with `judgeSessionConfig` entries and the review loop runs
- **THEN** every judge session and every human-escalation-probe session receives the entries in declared order before its prompt

#### Scenario: Removed model field is a type error

- **WHEN** a consumer shim configures the removed `model` or `judgeModel` field and runs `ptah check`
- **THEN** check reports a type error naming the unknown field, steering the consumer to the `sessionConfig` entry form

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
