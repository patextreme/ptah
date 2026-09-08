# Scripting Delta: add-env-reads

## MODIFIED Requirements

### Requirement: Sandboxed Luau environment
Scripts SHALL execute in a sandboxed Luau environment exposing only: `string`, `table`, `math`, `utf8`, `bit32`, `buffer`, `os.time`, `os.clock`, `os.getenv`, `print`, and a restricted `coroutine` table containing only `yield` (retained because the embedded async runtime requires it; all coroutine scheduling primitives MUST remain absent). The ambient environment MUST NOT expose file I/O, network, or debug facilities, and MUST NOT expose subprocess execution as a global (no `os.execute`, no `io`). Process execution reaches scripts only through the injected `ptah.exec` capability, specified by the `shell-exec` capability; scripts have no other host filesystem or network access beyond driving agents and `ptah.exec`. Environment reads reach scripts only through the sandboxed `os.getenv`, specified by the Environment variable reads requirement.

#### Scenario: Sandboxed globals
- **WHEN** a script accesses `io`, `os.execute`, `debug`, or `coroutine.create`
- **THEN** the access resolves to nil (or raises an error on call) because the globals are absent

#### Scenario: Print passthrough
- **WHEN** a script calls `print("hello")`
- **THEN** the line is written to ptah's standard output unmodified, without session prefixes

## ADDED Requirements

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
