## Context

The pr-review-loop component's `Config` exposes `reviewInstructionFile`:
a repo-relative path interpolated into one fixed prompt sentence ("Use the
review instruction at {path}…"); the sandboxed script never reads,
validates, or resolves the file — the work agent opens it. The default
branch already inlines text (the built-in default, a ~4.5KB persona data
module), so inline delivery is proven. The archived
`2026-09-04-pr-review-instruction-contract` change deferred an inline-text
config field on an evidence rule; this change revisits it — see proposal.md
(Why). Settled in a grilling session; the decisions below are that shared
understanding, recorded so the apply phase does not re-litigate them.

## Goals / Non-Goals

**Goals:**

- One config knob, `reviewInstruction: string?`, whose meaning is "the
  entire reviewer instruction text".
- One review-prompt template used for any instruction (configured or
  built-in default), keeping the classification enforcement ask in both
  modes (archived Decision 2).
- The pointer pattern (text referencing a repository document) documented
  as the recommended long/pinned form.
- Vocabulary: **Reviewer instruction** as the canonical term (CONTEXT.md
  entry already landed with this change).

**Non-Goals:**

- No delta/compose semantics over the default, and no promised
  default-composition surface (a consumer requiring
  `default-instruction.luau` to build default+delta text is emergent
  behavior we neither document nor forbid).
- No taxonomy parameterization, no structured verdicts — unchanged from
  the archived change.
- No compatibility alias for `reviewInstructionFile`.

## Decisions

1. **Replace semantics, `nil`-only fallback.** A configured
   `reviewInstruction` fully replaces the built-in default; only a nil
   value selects the default (the `~= nil` idiom the file field used).
   *Alternative:* delta semantics ("default, plus X") — rejected: it makes
   every configured instruction implicitly depend on default content that
   is versioned behavior (archived Decision 6 — a moving base), and
   renders the spec's precedence scenarios meaningless. *Alternative:*
   falsy/empty-string fallback — rejected: an empty string is a
   misconfiguration that should fail loudly, not silently degrade to the
   default.
2. **Clean break, no alias.** The direct form subsumes the file form, so
   two knobs for one slot is the two-ways-to-do-it trap. The break
   surfaces as a `ptah check` type error in consumer shims — the
   consumption README's declared compatibility gate — on a deliberate
   mount-ref bump. This repo's dogfood shims run default mode; nothing
   in-repo migrates.
3. **Single prompt template.** Delete the file branch; the default
   branch's wrapper ("Use the following review instruction:\n\n{text}\n
   \n---\n\nUsing this instruction, review PR {url}…") serves any
   instruction, with the blocking/non-blocking ask appended in both modes.
   *Alternative:* detect pointer-style text and special-case its framing —
   rejected: undetectable in general, and special-casing text we cannot
   reliably recognize is worse than a constant wrapper.
4. **Pointer pattern is documentation, not mechanism.** Long or
   repo-pinned instructions point at a versioned markdown document via one
   shim line; the component treats such text identically to any other.
   The README notes the honest trade: configured text is inlined into
   every iteration's prompt (up to `maxIterations` per run) — fine at the
   default's ~4.5KB, and the pointer pattern is the escape hatch for
   longer instructions.
5. **Spec delta shape.** Keep the `PR review instruction contract`
   requirement header and existing scenario names stable (openspec deltas
   locate requirements by header; rename churn buys nothing); reword
   bodies to the new field and term; strengthen the precedence scenario's
   THEN from "the path reaches the agent" to "the configured text is
   inlined and the default's classification directive is absent"
   (strictly stronger, testable against the mock); add one scenario
   pinning the pointer-pattern documentation requirement.
6. **No ADR.** The flip is cheap to reverse (a config-surface tweak), so
   it fails the hard-to-reverse test; the supersession of the archived
   Non-Goal is recorded here and in proposal.md.

## Risks / Trade-offs

- [A consumer's inline instruction is very long, bloating every
  iteration's prompt] → the README's pointer-pattern guidance is the
  documented answer; the default's size proves ordinary instructions are
  fine inlined.
- [A pointer-style instruction is phrased so vaguely the agent
  summarizes instead of reading the referenced document] → the component's
  constant wrapper ("Use the following review instruction:") frames
  whatever text it gets; agent reliability here is no worse than the old
  file mode's single interpolated sentence.
- [Consumers with a configured `reviewInstructionFile` hit a type error
  on bump] → intended: the error names the field and the accepted type;
  the fix is a one-line shim change, and the mount-ref pin means bumps
  are deliberate.
- [Default-composition consumers appear and want sanctioned surface] →
  revisit on evidence (the repo's extract-on-evidence rule); the declared
  contract makes that extension additive.

## Migration Plan

Single-sided: rename the field in the component, README, spec (synced at
archive), and tests. Rollback is the inverse rename; no data, wire format,
or stored state is involved.
