# Scripting Specification

## Purpose

Defines the Luau scripting environment embedded in ptah: the sandboxed standard library, module resolution, the `ptah` API namespace, concurrency primitives, and error/cancellation semantics observable by script authors.

## Requirements

### Requirement: Sandboxed Luau environment
Scripts SHALL execute in a sandboxed Luau environment exposing only: `string`, `table`, `math`, `utf8`, `bit32`, `buffer`, `os.time`, `os.clock`, `os.getenv`, `print`, and a restricted `coroutine` table containing only `yield` (retained because the embedded async runtime requires it; all coroutine scheduling primitives MUST remain absent). The ambient environment MUST NOT expose file I/O, network, or debug facilities, and MUST NOT expose subprocess execution as a global (no `os.execute`, no `io`). Process execution reaches scripts only through the injected `ptah.exec` capability, specified by the `shell-exec` capability; scripts have no other host filesystem or network access beyond driving agents and `ptah.exec`. Environment reads reach scripts only through the sandboxed `os.getenv`, specified by the Environment variable reads requirement.

#### Scenario: Sandboxed globals
- **WHEN** a script accesses `io`, `os.execute`, `debug`, or `coroutine.create`
- **THEN** the access resolves to nil (or raises an error on call) because the globals are absent

#### Scenario: Print passthrough
- **WHEN** a script calls `print("hello")`
- **THEN** the line is written to ptah's standard output unmodified, without session prefixes

### Requirement: Environment variable reads
The sandboxed `os` table SHALL provide `os.getenv(name)` returning the value of the named variable of ptah's environment, or `nil` when the variable is unset. A variable explicitly set to the empty string SHALL return the empty string — unset and set-to-empty are distinguishable. Reads SHALL observe a snapshot of ptah's environment taken once when the run starts. `os.getenv` SHALL be the only environment-read surface in the sandbox; the environment MUST NOT be enumerable (no listing of names) and scripts MUST NOT be able to modify ptah's environment. `os.getenv` SHALL raise a Lua error when called without a string argument. Variables whose values are not valid UTF-8 SHALL read as unset.

#### Scenario: Set variable
- **WHEN** a script calls `os.getenv("PTAH_ENV_PROBE")` and that variable is set to `"x"` in ptah's environment
- **THEN** the call returns `"x"`

#### Scenario: Unset variable reads as nil
- **WHEN** a script calls `os.getenv("PTAH_ENV_ABSENT")` for a variable ptah's environment does not carry
- **THEN** the call returns `nil`

#### Scenario: Empty string is distinct from unset
- **WHEN** a script calls `os.getenv("PTAH_ENV_EMPTY")` for a variable set to the empty string
- **THEN** the call returns `""` rather than `nil`

#### Scenario: Argument must be a string
- **WHEN** a script calls `os.getenv()` or `os.getenv(42)`
- **THEN** a Lua error is raised

#### Scenario: No mutation surface
- **WHEN** a script accesses `os.setenv` or any other environment-mutation global
- **THEN** the access resolves to nil (or raises an error on call) because the global is absent

### Requirement: Relative module resolution
Scripts SHALL be able to `require` modules by relative path from the requiring file's directory (e.g. `require("./lib/pipeline")`, resolving `.luau` files). Relative paths resolve without a boundary: a require MAY traverse out of the entry script's directory (e.g. `require("../shared/helper")`) to any module reachable by relative path. Non-relative require strings (absolute paths, bare module names, aliases) MUST be rejected with a Lua error.

#### Scenario: Sibling module
- **WHEN** a script at `main.luau` requires `./lib/util` and `lib/util.luau` exists
- **THEN** the module is loaded and its return value provided; a second require of the same path returns the cached module

#### Scenario: Module outside the entry script's directory
- **WHEN** a script at `workflow-1/main.luau` requires `../shared/helper` and `shared/helper.luau` exists as a sibling of `workflow-1/`
- **THEN** the module is loaded and its return value provided exactly as an in-directory module would be

#### Scenario: Missing module
- **WHEN** a script requires a path that does not resolve to an existing `.luau` file
- **THEN** the require call raises a Lua error naming the unresolved path

#### Scenario: Non-relative require string rejected
- **WHEN** a script requires an absolute path (`require("/etc/x")`) or a bare module name (`require("shared/helper")`)
- **THEN** the require call raises a Lua error stating that only `./` and `../` paths are allowed

### Requirement: Agent and session API
The `ptah` namespace SHALL provide `ptah.agent(name_or_spec)` returning an agent factory, and `agent:session(options)` returning a session object. Each `session()` call creates an independent session with its own agent subprocess. Session options SHALL accept `cwd` (resolved relative to the invocation directory), `id` (label used in output attribution, defaulting to `s1`, `s2`, … per agent), `mcpServers`, and `resultSchema` (a JSON Schema expressed as a Luau table; the option's semantics are specified by the typed-results capability). Two `ptah.agent` calls for the same name SHALL return independent factory objects.

#### Scenario: Session creation
- **WHEN** a script calls `ptah.agent("claude"):session({ id = "reviewer" })`
- **THEN** a session labeled `claude/reviewer` exists and is ready to prompt

#### Scenario: Default session labels
- **WHEN** two sessions are created without `id` from the same agent factory
- **THEN** they are labeled `s1` and `s2` respectively in output attribution

#### Scenario: Independent factories
- **WHEN** `ptah.agent("claude")` is called twice with the same name and each factory creates a session
- **THEN** the factories keep independent session counters: both first sessions are labeled `claude/s1`

#### Scenario: Unknown agent name
- **WHEN** `ptah.agent("nope")` is called and `nope` exists in no registry
- **THEN** a Lua error is raised naming the unresolved agent

### Requirement: Prompt returns a result table
`session:prompt(text, options?)` SHALL send one prompt turn and return a table with `text` (the turn's last agent message), `stopReason` (`"end_turn"`, `"max_tokens"`, `"max_turn_requests"`, `"refusal"`, or `"cancelled"`), `usage` (`input`, `cacheRead`, `cacheWrite`, `output` token counts, zero when unreported), and `result` (the turn's last accepted typed submission converted to a Luau value; `nil` when the session declared no contract or the turn had no accepted submission — the field's semantics are specified by the typed-results capability). The result table SHALL be directly string-coercible to `text` via `__tostring`. Options SHALL accept `timeoutMs`. Prompt turns on a single session SHALL be serialized: a `prompt` call issued while a turn is in flight on that session SHALL wait for that turn to complete before its own turn begins, so turns never interleave on one session. Distinct sessions are unaffected.

`text` SHALL be the last agent message of the turn: the final contiguous run of agent message text, where tool-call activity (`tool_call` and `tool_call_update` updates) terminates the current message run. When a turn ends with no message after its last tool-call activity, `text` SHALL fall back to the previous non-empty message run of that turn; when a turn produces no agent message at all, `text` SHALL be the empty string. When a turn completes with `stopReason == "cancelled"`, `text` SHALL be the empty string. Text from one turn SHALL never appear in a subsequent turn's `text` on the same session, whatever the previous turn's outcome. Streaming display of intermediate messages is unaffected: every message chunk is still surfaced by the live renderer as it arrives.

#### Scenario: Successful turn
- **WHEN** `local r = s:prompt("hi")` completes normally
- **THEN** `r.text` is the agent's final message, `tostring(r)` equals `r.text`, and `r.stopReason == "end_turn"`

#### Scenario: Last message after tool use
- **WHEN** a turn streams message A, then tool-call activity, then message B, and the turn completes
- **THEN** `r.text` equals message B and does not contain message A, while the run's streaming output showed both messages

#### Scenario: Turn ends on tool activity
- **WHEN** a turn streams message A, then tool-call activity, and completes with no message after it
- **THEN** `r.text` equals message A

#### Scenario: Cancelled turn has empty text
- **WHEN** a turn streams partial message text and is then cancelled (`stopReason == "cancelled"`)
- **THEN** `r.text` is the empty string

#### Scenario: No text leaks across turns
- **WHEN** a turn times out or is cancelled after streaming partial text, and the next prompt turn on the same session completes with message B
- **THEN** the next turn's `r.text` equals message B exactly, with no prefix from the aborted turn

#### Scenario: Timeout is an error
- **WHEN** `s:prompt("...", { timeoutMs = 50 })` exceeds its timeout
- **THEN** the turn is cancelled via `session/cancel` and the call raises a catchable Lua timeout error

#### Scenario: Turns on one session serialize
- **WHEN** a second `s:prompt(...)` is called while a turn is in flight on the same session
- **THEN** the second turn begins only after the in-flight turn completes

### Requirement: Cancellation is control flow, not failure
`session:cancel()` SHALL be callable while another task is blocked in `prompt` on that session; it sends `session/cancel`, and the awaiting `prompt` returns normally with `stopReason = "cancelled"` rather than raising.

#### Scenario: Watchdog cancel
- **WHEN** task A is blocked in `s:prompt(...)` and task B calls `s:cancel()`
- **THEN** task A's `prompt` returns a result with `stopReason == "cancelled"` and no error is raised

### Requirement: Task and concurrency primitives
The `ptah` namespace SHALL provide: `ptah.spawn(fn)` returning a Task object with `:await()`, `ptah.join({task, ...})` waiting for all tasks, `ptah.parallel(items, fn, options?)` running `fn` per item with optional `concurrency` limit (default unlimited) and returning per-item outcome entries, and `ptah.sleep(ms)`. Awaiting an errored task SHALL re-raise its error at the await site. `ptah.parallel` results SHALL carry each item's success value or error without throwing wholesale. A task error is delivered when observed via `:await()`, `join`, or carried in `ptah.parallel` results, whether or not the script catches or inspects it; a task error never delivered by script end SHALL fail the run (error to stderr, non-zero exit).

#### Scenario: Parallel fan-out
- **WHEN** `ptah.parallel({1,2,3}, function(i) return agent:session():prompt("q"..i) end)` runs
- **THEN** all three turns execute concurrently, each on its own session, and results arrive in item order

#### Scenario: Concurrency cap
- **WHEN** `ptah.parallel(items, fn, { concurrency = 2 })` runs with 5 items
- **THEN** at most 2 `fn` invocations are in flight simultaneously

#### Scenario: Contained task error
- **WHEN** one of three spawned tasks raises and the script joins all three
- **THEN** the two successful values are available and the failed task's error is reported for its entry; other tasks are unaffected

#### Scenario: Error re-raised at await
- **WHEN** a spawned function raises an error and `task:await()` is called
- **THEN** the original error is re-raised at the await call site

### Requirement: Runtime helpers
The `ptah` namespace SHALL provide `ptah.log(msg)` printing a `[ptah]`-prefixed diagnostic line to standard output, `ptah.exit(code)` terminating the run, `ptah.sleep(ms)` yielding the current task for the duration, and `ptah.version` (read-only version string).

#### Scenario: Log attribution
- **WHEN** a script calls `ptah.log("starting")`
- **THEN** output shows `[ptah] starting` on its own line

#### Scenario: Sleep yields
- **WHEN** task A calls `ptah.sleep(100)` while task B prompts
- **THEN** task B progresses during A's sleep

### Requirement: JSON module
The `ptah` namespace SHALL provide `ptah.json.parse(string)` returning the decoded value as Luau data (arrays as tables with consecutive integer keys starting at 1, objects as string-keyed tables, `null` as `nil`), raising a Lua error on malformed input; and `ptah.json.stringify(value, { indent?: number })` returning the encoded JSON string. The module performs no I/O.

#### Scenario: Round trip
- **WHEN** a script calls `ptah.json.parse('{"a":[1,2]}')` and stringifies the result with `ptah.json.stringify(v, { indent = 2 })`
- **THEN** the output is valid JSON encoding `{"a": [1, 2]}` with two-space indentation

#### Scenario: Malformed input raises
- **WHEN** a script calls `ptah.json.parse("{oops")`
- **THEN** a Lua error is raised and can be caught with `pcall`

#### Scenario: Decoding command output
- **WHEN** a script calls `ptah.exec("printf '[{\\"n\\":1}]'")` and passes `r.stdout` to `ptah.json.parse`
- **THEN** the result is a table whose first element is `{ n = 1 }`
