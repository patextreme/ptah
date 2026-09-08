## MODIFIED Requirements

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
