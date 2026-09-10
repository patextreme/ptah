## ADDED Requirements

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

## MODIFIED Requirements

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
human-escalation-probe session. The `model` and `judgeModel` config fields
SHALL NOT exist: a model choice is an ordinary `sessionConfig` entry, and
the entry order is the consumer's `setConfig` order.

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
