# Proposal: add-env-reads

## Why

Scripts have no way to read environment variables. The sandbox rebuilds `os`
with only `time` and `clock`, so Luau tooling ported into ptah breaks on a
call (`os.getenv`) that works everywhere else, and scripts cannot branch on
ambient knobs (`CI`, `DEBUG`, workspace paths) without laundering them
through agent-registry `${VAR}` interpolation — which only composes agent
argv. Scripts are already trusted code driving agents with the user's full
authority; a read-only window onto ptah's environment is the same
blast-radius class as `os.time`, and the sandbox's charter is limiting the
blast radius of bugs, not malice.

## What Changes

- The sandboxed `os` table gains `getenv`: `os.getenv(name) -> string?`,
  the standard Luau contract — value for a set variable, `nil` when unset
  (including variables dropped for being non-UTF-8), `""` for a variable
  explicitly set to the empty string, exactly one string argument. There is
  no `ptah.env`; `os.getenv` is the single environment-read surface.
- Reads observe a snapshot of ptah's environment captured once at the
  composition root and injected into the runtime like every other
  capability. ptah-luau performs no ambient environment reads in bindings;
  scripts cannot enumerate the environment or mutate it (no `setenv`, no
  env options on `ptah.exec`, no change to `${VAR}` interpolation).
- The embedded type definitions mirror the new member; the README sandbox
  enumeration, snapshot/read-only wording, and trusted-scripts note are
  updated; `skills/ptah/SKILL.md` mentions the surface; a bundled example
  exercises it.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `scripting`: the "Sandboxed Luau environment" enumeration gains
  `os.getenv`, and a new "Environment variable reads" requirement pins the
  observable contract (nil-unset vs empty-string, snapshot semantics,
  single-surface, no enumeration or mutation).
- `type-definitions`: the "Definitions model the sandbox" requirement's
  `os` mirror gains `getenv`, keeping editor-approved code unable to reach
  a global that would fail at runtime (and vice versa).

## Impact

- `crates/ptah-luau`: `sandbox.rs` binds `getenv` next to `time`/`clock`;
  `state.rs` carries the snapshot in `RunConfig`/`RuntimeState`.
- `crates/ptah-cli`: `cli.rs` captures `env::vars()` at composition.
- `.ptah/ptah.d.luau`: `declare os` gains
  `getenv: (name: string) -> string?`; the runtime probe test (spec-pinned
  by type-definitions "Definitions stay synchronized") exercises it.
- Docs: `README.md`, `skills/ptah/SKILL.md`, new `examples/env.luau` with
  its `examples.rs` test entry.
- `crates/ptah-core` is untouched — no new port. Env-read is data capture,
  not an operation needing an adapter; the funded-port set stays closed and
  `deps_guard` is unaffected.
- CONTEXT.md already gained the "ptah's environment" glossary term during
  the design session that produced this change.
