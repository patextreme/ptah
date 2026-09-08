## MODIFIED Requirements

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
