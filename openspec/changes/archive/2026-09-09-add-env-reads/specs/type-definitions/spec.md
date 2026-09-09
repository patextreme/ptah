# Type Definitions Delta: add-env-reads

## MODIFIED Requirements

### Requirement: Definitions model the sandbox
The definitions SHALL shadow the trimmed globals the runtime provides: `os` restricted to `time`, `clock`, and `getenv` (typed `getenv: (name: string) -> string?`, returning `nil` for unset variables), `coroutine` restricted to `yield`, and `loadstring` and `collectgarbage` declared as nil.

#### Scenario: Removed global flagged
- **WHEN** a script analyzed with the definitions calls `os.date`, `coroutine.create`, or `loadstring`
- **THEN** analysis reports a type error instead of the call being accepted and failing at runtime
