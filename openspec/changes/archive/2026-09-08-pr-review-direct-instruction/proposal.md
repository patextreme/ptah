## Why

`reviewInstructionFile` is a file path with no file mechanism behind it: the
sandboxed script never reads the file, never validates it, and never resolves
the path — the field is a string interpolated into one fixed prompt sentence
that tells the *agent* to open the document. Direct instruction text is the
same mechanism with strictly more expressive power, and it collapses the
component's two prompt branches into one. The archived
`pr-review-instruction-contract` change considered a `reviewInstruction:
string` field and deferred it on an evidence rule ("repos that won't author
a file won't author text"); that prediction is now revisited by the
maintainer — pointer-style text subsumes the file form, so the file field
adds a knob without adding power.

## What Changes

- **BREAKING**: the pr-review-loop `Config` field `reviewInstructionFile:
  string?` is replaced by `reviewInstruction: string?` — the reviewer
  instruction as direct text. No deprecated alias; the clean break surfaces
  as a `ptah check` type error in consumer shims (the declared compatibility
  gate) on their next deliberate mount-ref bump. This repo's own dogfood
  shims run default mode, so nothing in-repo migrates.
- Replace semantics: a configured `reviewInstruction` fully replaces the
  built-in default; only `nil` selects the default (an empty string is a
  loud misconfiguration, not a silent fallback).
- The two-branch review ask in `component.luau` collapses to one template —
  the default branch's inline wrapper used for any instruction — with the
  classification enforcement ask retained in both modes (archived Decision 2).
- Vocabulary: the canonical term becomes **Reviewer instruction**
  (`CONTEXT.md` updated during planning); "instruction document" demotes to
  the *pointer pattern* — the documented convention for long or repo-pinned
  instructions: one shim line pointing at a versioned markdown document.
- Spec rewording under the existing `PR review instruction contract`
  requirement: headers and scenario names stay stable, bodies follow the new
  field and term, and the precedence scenario's THEN strengthens from "the
  path reaches the agent" to "the configured text is inlined into the review
  prompt and the default's classification directive is absent".
- Default-composition (requiring `default-instruction.luau` to build
  default+delta text) stays emergent, not promised surface.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `factory-components`: the `PR review instruction contract` requirement's
  config surface changes from an instruction-document path to direct
  reviewer-instruction text (replace semantics, `nil`-only fallback, single
  prompt template, pointer-pattern convention documented).

## Impact

- `factory-components/components/pr-review-loop/component.luau` — Config
  type/doc comment, branch collapse.
- `factory-components/components/pr-review-loop/README.md` — contract
  section, built-in-default section, environment requirements (document
  bullet goes conditional), config example, pointer-pattern convention.
- `crates/ptah-cli/tests/factory_components.rs` — precedence test flips to
  text-inlined + no-`BLOCKING`; type-error fixture renames the field.
- `CONTEXT.md` — Reviewer instruction entry (already landed with this
  change's planning).
- `openspec/specs/factory-components/spec.md` — synced at archive.
- Supersedes the archived `2026-09-04-pr-review-instruction-contract`
  Non-Goal "no inline instruction-text config"; no ADR (the flip is cheap to
  reverse, and the supersession is recorded here and in design.md).
