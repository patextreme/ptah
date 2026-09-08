# Design: add-env-reads

## Context

The sandbox rebuilds `os` from scratch in `crates/ptah-luau/src/sandbox.rs`,
keeping only `time` and `clock`; `os.getenv` is simply never set. The
runtime already has one capability-injection seam: optional powers arrive
via `RunConfig` (`crates/ptah-luau/src/state.rs`), populated at the
composition root (`crates/ptah-cli/src/cli.rs`) — `ptah.exec` reaches
bindings this way through the `ProcessRunner` port. One pre-existing
carve-out: `${VAR}` interpolation does a live `std::env::var` inside
`bindings.rs` (`interp_lookup`) at `ptah.agent()` call time. The type
definitions (`.ptah/ptah.d.luau`) mirror the trimmed globals under
`declare os`, and a runtime probe test exercises every member the defs
promise. See proposal.md for motivation.

## Goals / Non-Goals

**Goals:**

- `os.getenv(name) -> string?` available to scripts, semantics identical
  to standard Luau (nil for unset), reading a startup snapshot.
- The capability flows through the existing injection seam so ptah-luau
  gains no new ambient authority.
- Editor/analysis surface stays in lockstep with the runtime (defs mirror
  + probe test + example).

**Non-Goals:**

- No `ptah.env` API, no alias — one surface only.
- No `setenv`, no per-exec env options, no changes to `${VAR}`
  interpolation or agent env inheritance (those live in `agent-registry`
  and `shell-exec` specs and are untouched).
- No enumeration (`os.getenv()` with no argument is an error, not a dump).

## Decisions

### D1: Surface — restore `os.getenv` rather than add `ptah.env`

Alternatives: a `ptah.env(name)` function (namespace purism) or a
metatable `env` table. Chose the stdlib name because the porting charter
(ported scripts must work unmodified) is served for free; every settled
semantic (nil-unset, one-string-arg, no write side) *is* the stdlib
contract, so nothing bespoke has to be specified or defended; and env-read
is the same class as `os.time`/`os.clock` — passive, read-only, standard
ambient state. The "world access arrives through capabilities injected at
the composition root" principle is about *powers* (subprocess execution);
passive stdlib state was already outside it. Costs, accepted: the API home
is outside the `ptah` namespace (documented in the README sandbox
enumeration), and the stdlib shape invites `os.date`-style asks later
(scope discipline, same as today).

### D2: Injected snapshot, not an ambient read in the binding

`cli.rs` captures `std::env::vars().collect()` into a
`BTreeMap<String, String>` on `RunConfig`; `sandbox.rs` binds an
`os.getenv` closure over the snapshot right where `time`/`clock` are set.
No `std::env` call anywhere in the script-facing binding path.
Alternatives: calling `std::env::var` directly in the binding (smallest
diff, and `interp_lookup` already crosses that line — rejected because a
script-facing API is a more visible commitment than an internal
interpolation hook, and injection keeps the architecture sentence
literally true); or a new core port (`EnvSource`, the full `ptah.exec`
treatment — rejected: env-read is data capture, not an operation needing
an adapter; the funded-port set in `crates/ptah-core/src/ports.rs` stays
closed, and `deps_guard` stays untouched).

### D3: Snapshot timing and the interpolation asymmetry

`os.getenv` observes startup state; `${VAR}` interpolation stays
live-at-`ptah.agent()`-call (existing behavior, spec-pinned by
`agent-registry`). These cannot observably diverge: nothing in ptah
mutates its process environment mid-run, and scripts have no mutation
surface (D4). Chosen over unifying on live reads, which would reintroduce
the ambient-read-in-binding that D2 excludes for no observable gain.

### D4: Read-only fence, no enumeration

No `setenv`-shaped global, no listing API, `os.getenv()` with a missing
or non-string argument raises. Secrets in the environment are now
script-readable; this is explicitly accepted under the README's
trusted-scripts threat model ("the sandbox limits the blast radius of
bugs, not malice") and gets a documenting clause rather than a redaction
mechanism. An allowlist was considered and rejected as over-engineering
against that threat model.

### D5: Non-UTF-8 values read as unset

`std::env::vars()` skips non-UTF-8 entries, so they read as `nil`. The
lossless alternative (`env::vars_os` into Luau byte strings) was rejected:
every downstream consumer (prompts, `ptah.json`, agent argv) wants UTF-8,
and "reads as unset" is a predictable failure mode.

## Risks / Trade-offs

- [Two homes for "world" access (`ptah.exec`, `os.getenv`)] → README
  sandbox section enumerates the `os` surface explicitly next to the
  capability story, so the split is documented rather than discovered.
- [Readers assume live or mutable env] → README states snapshot + read-only
  semantics in the same breath as the enumeration.
- [Probe/defs drift if `getenv` is added to one but not the other] → the
  spec-pinned probe test ("Definitions stay synchronized") fails closed;
  the tasks add the probe exercise next to the defs edit.

## Migration Plan

Purely additive; no behavior changes for existing scripts (any script
relying on `os.getenv` being absent was already erroring on nil-call).
Rollback is reverting the change. No wire, registry, or config format
changes.
