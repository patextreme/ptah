# Scripting Capability Delta

## ADDED Requirements

### Requirement: Human ask primitive
The `ptah` namespace SHALL provide `ptah.ask(opts)` where `opts` is a table accepting `prompt` (required string) and `details` (optional string), and no other option in v1. The call blocks only the invoking coroutine (root chunk or task body alike), returns the ask result whose semantics are specified by the ask capability, and raises a usage error when `prompt` is missing or not a string. `ptah.ask` is callable from anywhere a yield is legal, including inside `ptah.spawn` and `ptah.parallel` callbacks.

#### Scenario: Ask from a task body
- **WHEN** a script calls `ptah.spawn(function() return ptah.ask({ prompt = "q" }) end)` and the human answers
- **THEN** the task's `:await()` returns the ask result

#### Scenario: Missing prompt raises a usage error
- **WHEN** a script calls `ptah.ask({ details = "context" })`
- **THEN** the call raises a catchable error naming the required `prompt` field

#### Scenario: Blocking is per-coroutine
- **WHEN** task A blocks in `ptah.ask(...)` and task B calls `ptah.sleep(10)` then finishes
- **THEN** task B completes while A is still parked in the ask
