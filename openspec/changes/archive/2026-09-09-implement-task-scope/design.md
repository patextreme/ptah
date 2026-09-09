## Context

The openspec component (`factory-components/components/openspec/component.luau`)
runs every operation through one convergence skeleton (`drive`): prompt
the work agent, judge the final text with `std/predicate`, probe for
human input, resolve, repeat — bounded by the iteration cap. The judge
sees only the predicate string and the work session's final text; it
never sees the work prompt and has no repo access. The component is
sandboxed Luau with no file I/O: it cannot read the change's tasks.md,
so any task subset must be expressed as text the agents interpret.

The per-operation data of `drive` is four strings (`accepted`,
`probePrompt`, `humanPredicate`, `resolvePrompt`) plus the work prompt
itself; the pause vocabulary (probe/human/resolve) is shared verbatim
by groom, implement, and verify. The only consumer entry point is the
hand-edited shim `.ptah/workflows/openspec/main.luau`. Offline coverage
lives in `crates/ptah-cli/tests/factory_components.rs`, where the mock
agent echoes prompts back into the judge payload, so both the work
prompt and the judge predicate are assertable.

See proposal.md for motivation (incremental landing of large changes;
today's loop actively fights partial runs via the resolve step).

## Goals / Non-Goals

**Goals:**

- `implement` accepts an optional free-text task scope; completion is
  judged against the scope.
- Nil scope is byte-identical to today's behavior (prompts, predicate,
  log lines) — existing callers and tests unchanged.
- Unresolvable scopes fail deterministically through the existing
  human-escalation path rather than through agent improvisation.

**Non-Goals:**

- Structured selectors (group numbers, task ids): group numbering is a
  markdown convention the component cannot validate, and free text also
  covers descriptive subsets ("the env-reads tasks", "skip docs").
- Scopes on `groom`/`verify`: verify ends in sync-and-archive, which a
  partially-implemented change cannot take; groom is proposal-level.
- Guarding against out-of-scope edits in the judge: the judge's only
  evidence is the agent's own final text, so a judged guard adds false
  rejections with no enforcement gain. The guard is prompt-side honesty
  ("leave all other tasks pending").
- Component-side validation of the scope against tasks.md (impossible:
  no file I/O) and any change to the openspec skills themselves.

## Decisions

**Optional positional string, not an options table.**
`implement: (self, change: string, scope: string?) -> string`. Matches
the components' flat positional per-call-data convention
(`pr-review-loop.review(prUrl)`); no second per-call knob is
anticipated. Alternative rejected: `implement(change, { scope = ... })`
future-proofs more knobs but is ceremony for one, and switching later
is a breaking signature change either way — acceptable to make then,
not now.

**Scope interpolates into exactly two places: work prompt and accepted
predicate.** The judge never sees the work prompt, so the scope must
ride inside the predicate or the verdict cannot be scoped.

- Scoped work prompt (the Q5+Q7 wording from the design session):

  ```
  Implement the pending tasks of the openspec change named {change} using the openspec-apply-change skill. The task scope for this run is: {scope}. Treat the tasks matching the scope as the entire job: implement those, leave all other tasks pending, and end each pass either with the scoped tasks implemented or paused with a stated reason, as the skill defines those states. If the task scope matches no tasks, end the pass stating that; do not guess or substitute.
  ```

- Scoped accepted predicate:

  ```
  All tasks in the following task scope are implemented: "{scope}"
  ```

- Nil scope falls through to today's strings byte-for-byte.

The pause texts (`probePrompt`, `humanPredicate`, `resolvePrompt`) stay
untouched: they are scope-agnostic pause vocabulary, and the resolve
text's "continue implementing the remaining tasks" correctly pushes an
agent that paused before finishing its scope.

**Unresolvable scope is a prompt clause, not new machinery.** The
"matches no tasks → state it, don't guess" sentence in the work prompt
routes garbage scopes into the existing pause → probe → human-escalation
path, which already errors with the probe excerpt. No new error type.

**Scoped log line.** `ptah.log` gains the scope when present
(`implementing change {change} (task scope: {scope})`) so the shim's
console shows what a run was bounded to; nil scope keeps today's line.

**Glossary term: "task scope".** Added to `CONTEXT.md` during the
design session (avoid: filter — the component cannot see the tasks;
instruction — a scope redefines completion, an instruction does not).

## Risks / Trade-offs

- [Judge accepts scoped completion on the agent's word alone] → Inherent
  to the existing loop (already true for whole-change completion); the
  scoped predicate narrows the claim being judged rather than widening it.
- [Ambiguous scope text resolved differently than the caller meant] → The
  judge holds the run to the same ambiguous text, and the dead-end clause
  prevents silent substitution; genuinely ambiguous-but-matching scopes
  are the caller's responsibility, matching the component's
  free-text-per-call-data convention.
- [Out-of-scope tasks implemented anyway] → Prompt-side instruction only
  (see Non-Goals); a judge-side guard would have no teeth and would tax
  the single-boolean predicate.

## Migration Plan

Additive optional parameter on mounted-source library code: consumers
pick it up on their next mount sync; existing two-argument call sites
compile and behave identically. Rollback is reverting the component,
README, and test edits — no persisted state, no wire format.

## Open Questions

(none)
