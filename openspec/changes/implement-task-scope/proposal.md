## Why

The openspec component's `implement` operation is all-or-nothing: it drives
task execution until *every* task of the change is implemented. The only
consumer entry point is a hand-edited shim (ptah scripts take no runtime
arguments), so there is no way to land a large change in deliberate
increments — group 1 today, the rest after review. Running a partial pass
today is not just unsupported, it is *fought by the loop*: a paused pass
with tasks remaining is judge-rejected and the resolve step pushes the
agent onward until the whole change is done or the cap kills the run.

## What Changes

- `implement` gains an optional task scope: `ops:implement(change, scope)`.
  The scope is free text describing the subset of tasks the run is
  responsible for (e.g. `"task group 1"`); completion — and the
  convergence loop's acceptance — is judged against the scope, not
  against the whole change.
- The scope interpolates into exactly two places: the work prompt (which
  tells the agent to treat the scoped tasks as the entire job and leave
  all other tasks pending) and the judge's accepted-predicate (which
  carries the scope text, since the judge never sees the work prompt).
- A scope that matches no tasks ends the pass stating that instead of the
  agent guessing; the existing human-escalation path then surfaces it as
  an operation error.
- `groom` and `verify` are unchanged (verify ends in archive, which a
  partially-implemented change cannot take; groom is proposal-level).
- A nil scope keeps today's behavior byte-for-byte: identical prompts,
  identical predicate, identical log lines.
- The term **task scope** is added to the repo glossary (`CONTEXT.md` —
  already captured during the design session that produced this change).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `factory-components`: the openspec component requirement's `implement`
  operation gains the optional task-scope argument with scoped completion
  semantics; new scenarios cover scoped completion, scopeless
  compatibility, and the unresolvable-scope dead-end.

## Impact

- `factory-components/components/openspec/component.luau` — `Instance`
  type (`implement` gains `scope: string?`), implement body (prompt and
  predicate construction), scoped log line.
- `factory-components/components/openspec/README.md` — operations
  section documents the optional scope and its completion contract.
- `.ptah/workflows/openspec/main.luau` — one usage-hint comment at the
  implement call site.
- `CONTEXT.md` — glossary entry (landed during the design session).
- `crates/ptah-cli/tests/factory_components.rs` — offline coverage: a
  scoped implement run (scope text asserted in both work prompt and judge
  predicate via the mock's prompt echo) and a nil-scope back-compat run.
- No crate code changes; the library is mounted source, so consumers
  pick the new parameter up on their next mount sync. Call sites passing
  only `change` are unaffected (optional positional parameter).
