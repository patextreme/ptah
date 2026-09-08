# Tasks: implement-task-scope

## 1. Component

- [x] 1.1 Extend the `Instance` type and `implement` body in
  `factory-components/components/openspec/component.luau`:
  `implement: (self: Instance, change: string, scope: string?) -> string`,
  with the scoped work prompt, scoped accepted predicate, and scoped log
  line exactly as pinned in design.md, and the nil-scope path producing
  today's strings byte-for-byte. Verify: a diff review shows the pause
  texts (`probePrompt`/`humanPredicate`/`resolvePrompt`) and the
  groom/verify operations untouched.
- [x] 1.2 Update `factory-components/components/openspec/README.md`:
  document `ops:implement(change, scope)` — the optional free-text task
  scope, scoped-completion contract, dead-end behavior for unresolvable
  scopes, and scopeless compatibility. Verify: README grep for
  "task scope" in the operations section; config section unchanged.

## 2. Offline coverage

- [x] 2.1 Add `openspec_component_implements_a_scoped_change` to
  `crates/ptah-cli/tests/factory_components.rs`: drive a scoped
  implement run through the mock and assert (via the mock's prompt echo
  into the judge payload) that the scope text reached both the work
  prompt and the accepted predicate. Verify:
  `cargo test --test factory_components openspec_component_implements_a_scoped_change`
  passes.
- [x] 2.2 Cover scopeless compatibility in the same file: the existing
  `openspec_component_implements_a_change` run (or an added nil-scope
  variant) asserts today's unscoped prompt and predicate strings still
  appear. Verify:
  `cargo test --test factory_components openspec_component_implements`
  passes with both cases.
- [x] 2.3 Cover the unresolvable-scope dead-end in the same file
  (folded in during verification): `openspec_component_implements_an_unresolvable_scope_fails`
  drives a scoped run whose judge rejects the scoped predicate and
  whose human probe escalates — asserting the dead-end clause reached
  the agent, the scoped predicate reached the judge, the operation
  failed with the escalation error, and no resolve prompt was issued.
  Verify:
  `cargo test --test factory_components openspec_component_implements_an_unresolvable_scope_fails`
  passes.

## 3. Consumer surface

- [x] 3.1 Edit `.ptah/workflows/openspec/main.luau`: add the usage-hint
  comment above `opsx:implement(changeName)`
  (`-- opsx:implement(changeName, "task group 1")`), together with the
  consumer-side restructure that landed in the same edit (absorbed
  during verification): pr-review-loop wiring removed, the workflow
  wrapped in a `processChange` helper that runs `implement-task-scope`
  then `add-env-reads` through groom/implement/verify and a commit
  session. Verify: grep finds the comment; the shim runs against the
  mock in its actual shape
  (`cargo test --test factory_components dogfood_openspec`).
- [x] 3.2 Confirm the glossary entry: `CONTEXT.md` carries the
  **Task scope** entry with the confirmed wording (landed during the
  design session). Verify: grep for "Task scope" in CONTEXT.md.

## 4. Guard

- [x] 4.1 Run the full offline suite. Verify: `cargo test` (and
  `cargo test --test factory_components` explicitly) passes; no
  crate code outside the test file changed (`git diff --stat` shows only
  the component, its README, the shim (comment plus restructure), the
  test file, CONTEXT.md, and this change's artifacts).
