# Ask Specification

## Purpose

Defines how workflows pause for human guidance: the `ptah.ask` primitive that suspends only the calling coroutine, the pluggable interaction provider that delivers questions (stdin first), operator-side provider selection including the `none` posture, and the serialization, attribution, and observability of asks.

## Requirements

### Requirement: Ask suspends only the calling coroutine
The scripting environment SHALL provide `ptah.ask(opts)` where `opts` is `{ prompt: string, details: string? }` (`prompt` required; no other option in v1). The call suspends only the coroutine that invoked it: other tasks, in-flight prompt turns, streaming output, and agent sessions SHALL continue to progress while an ask is pending. The call resolves when the human answers or aborts; v1 has no timeout option, so an ask blocks until answered, aborted, or the run itself ends. A pending ask at script end keeps the run alive exactly like an outstanding task. Agent sessions SHALL survive an ask: an in-flight turn completes normally while an ask is pending, and the same session serves further prompts after the ask resolves.

#### Scenario: Resume with an answer
- **WHEN** a script calls `local a = ptah.ask({ prompt = "Continue?" })` and a human answers `yes`
- **THEN** the call returns `{ action = "respond", text = "yes" }` and the script continues on the same coroutine

#### Scenario: Other work progresses during an ask
- **WHEN** task A blocks in `ptah.ask(...)` while task B awaits a prompt turn on another session
- **THEN** task B's turn completes and streams output while the ask is still pending

#### Scenario: ACP session survives an ask
- **WHEN** an ask is pending while a turn is in flight on a session, the turn completes, and the ask is then answered
- **THEN** the same session (same subprocess) serves a subsequent `session:prompt` without error

### Requirement: Response-or-abort result
`ptah.ask` SHALL return exactly one of `{ action = "respond", text: string }` or `{ action = "abort" }`. The response `text` SHALL be the answer as received from the provider, unprocessed (v1: one line; no multiline option). Abort is a normal result the workflow handles like any other value, not an error.

#### Scenario: Respond
- **WHEN** the human answers an ask with `ship it`
- **THEN** the result is `{ action = "respond", text = "ship it" }`

#### Scenario: Abort
- **WHEN** the human aborts an ask (per the active provider's abort gesture)
- **THEN** the result is `{ action = "abort" }` with no `text` field

### Requirement: Failures raise distinctly and never hang silently
`ptah.ask` SHALL raise a catchable Lua error — never return, and never suspend indefinitely without a resolution path — for each of these distinct conditions, each with its own stable message identifying the condition: interaction prohibited (the `none` posture was selected), no provider configured (nothing resolved the ask channel), provider failure (the provider itself failed), and end-of-input (the provider's input source closed with no gesture). Prohibited is an operator posture; no-provider is a configuration gap; the two SHALL NOT share a message.

#### Scenario: Prohibited raises
- **WHEN** the run resolved to `none` and the script calls `ptah.ask(...)`
- **THEN** the call raises an error identifying prohibited interaction, catchable with `pcall`

#### Scenario: No provider raises
- **WHEN** no provider was configured or detected and the script calls `ptah.ask(...)`
- **THEN** the call raises a different error identifying the missing provider configuration, naming the selection knobs

#### Scenario: pcall contains a provider failure
- **WHEN** the active provider fails mid-ask and the script wrapped the call in `pcall`
- **THEN** the pcall returns `false` plus the error and the script continues

### Requirement: Provider selection is an operator decision with precedence
The ask provider SHALL be selected by the operator, not the script, via exactly one active provider per run, resolved with the precedence `--ask` CLI flag > `PTAH_ASK` environment variable > project `[ask]` registry section > user `[ask]` registry section > automatic detection. Automatic detection SHALL select the `stdin` provider only when ptah's stdin and stdout are both terminals; otherwise, with no explicit selection, the run has no provider (asks raise no-provider at call time). The `none` value SHALL prohibit interaction outright: `ptah.ask` raises prohibited, while agent permission answering is entirely unaffected (`none` governs asking, not the headless permission posture). Provider selection SHALL NOT be settable from script code.

#### Scenario: Flag beats environment
- **WHEN** `ptah run --ask=stdin script.luau` runs with `PTAH_ASK=none` set
- **THEN** the stdin provider is active

#### Scenario: Environment beats project config
- **WHEN** a run has `PTAH_ASK=none` and the project config sets `[ask] provider = "stdin"`
- **THEN** interaction is prohibited

#### Scenario: Project beats user config
- **WHEN** the user config sets `[ask] provider = "none"` and the project config sets `[ask] provider = "stdin"`
- **THEN** the stdin provider is active

#### Scenario: Auto-detection on a terminal
- **WHEN** a run starts on an interactive terminal with no `--ask`, `PTAH_ASK`, or `[ask]` anywhere
- **THEN** the stdin provider is active without any configuration

#### Scenario: none leaves permissions alone
- **WHEN** a run resolves to `none` and an agent requests a permission
- **THEN** the permission is answered by the existing headless allow-all posture, unchanged

### Requirement: Concurrent asks serialize with attribution
Asks issued concurrently SHALL be delivered to the human one at a time in first-come-first-served order; prompts SHALL NOT overlap or interleave each other. Each ask SHALL be attributed with a per-run monotonic ask number starting at 1 and the entry script's basename, so the human can tell which run and script is asking. Asks SHALL NOT be attributed to agents or sessions (asking is workflow-level).

#### Scenario: Two concurrent asks queue
- **WHEN** two tasks call `ptah.ask` nearly simultaneously
- **THEN** the first-issued ask is prompted and resolved before the second is prompted

#### Scenario: Attribution names the script
- **WHEN** `workflow-1/main.luau` issues the run's second ask
- **THEN** the prompt is attributed `ask 2` carrying `main.luau`

### Requirement: stdin provider reads answers from stdin
The `stdin` provider SHALL render the ask (prompt, then details when present) to ptah's stdout per the render-logging contract, then read the answer as one line from ptah's stdin. An answer line of exactly `/abort` SHALL resolve `{ action = "abort" }`. On an interactive terminal, Ctrl-D at an empty prompt SHALL resolve `{ action = "abort" }` as well. When stdin is not a terminal and reaches end-of-input (the writer closed it), the ask SHALL raise the end-of-input error. The stdin provider SHALL work over non-terminal stdin whenever explicitly selected, so scripted runs can answer through a pipe.

#### Scenario: Line answer over a pipe
- **WHEN** a run started with `--ask=stdin` has its stdin piped and the writer sends `go ahead\n`
- **THEN** the ask resolves `{ action = "respond", text = "go ahead" }`

#### Scenario: /abort sentinel
- **WHEN** the human types `/abort` at an ask prompt
- **THEN** the ask resolves `{ action = "abort" }`

#### Scenario: Ctrl-D aborts on a terminal
- **WHEN** the human presses Ctrl-D at an empty ask prompt on an interactive terminal
- **THEN** the ask resolves `{ action = "abort" }`

#### Scenario: Closed pipe raises
- **WHEN** a piped run's stdin writer closes without sending an answer line
- **THEN** the ask raises the end-of-input error rather than resolving abort

### Requirement: Ask lifecycle is observable
Each `ptah.ask` call SHALL emit lifecycle events through the event sink: a requested event carrying the prompt and details, and a resolved event carrying the action and the response text (the full answer, for downstream sinks). An ask dropped by run teardown SHALL emit no resolved event — the run is ending, and cancellation is not a human outcome. Rendering of these events follows the render-logging contract.

#### Scenario: Events fire around an answered ask
- **WHEN** a script's ask is answered `retry`
- **THEN** a requested event (prompt, details) and a resolved event (`respond`, `retry`) are emitted to the sink, in that order

#### Scenario: Teardown emits no resolution
- **WHEN** the run is cancelled (SIGINT) while an ask is pending
- **THEN** no resolved event is emitted for the dropped ask

### Requirement: Cancellation tears down pending asks
An outer signal (SIGINT/SIGTERM) during a pending ask SHALL follow the existing run-cancellation contract unchanged: teardown runs, the pending ask's coroutine is dropped without an abort being delivered (abort is a human answer; cancel is process-level), and the process exits 128+signal (130/143). Ctrl-C SHALL NOT be repurposed as an ask abort gesture.

#### Scenario: Ctrl-C during an ask
- **WHEN** SIGINT arrives while a task is parked in `ptah.ask(...)`
- **THEN** the run tears down per the existing cancellation contract and exits 130, and the script never observes a result for the ask
