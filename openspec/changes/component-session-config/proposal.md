## Why

The workflow components can select a session's model (`model`/`judgeModel`) but
cannot tune any other ACP session config option — reasoning effort, thinking
level, agent-specific toggles — even though `setConfig` supports them all.
Consumers driving components against agents with dependent options (opencode
re-derives `effort` from every `model` set) also have no way to express the
application order that ptah's core API makes load-bearing.

## What Changes

- Add ordered session-config entries to the factory-components library: an
  entry is `{ id: string, value: string | boolean }` — the consumer's
  `setConfig` call sequence, as data.
- Both components (`openspec`, `pr-review-loop`) accept
  `sessionConfig: {Entry}?` for work sessions and
  `judgeSessionConfig: {Entry}?` for judge and human-escalation-probe sessions.
  Entries apply in declared array order after session creation, before the
  first prompt.
- **BREAKING**: the `model` and `judgeModel` config fields are removed from
  both components — the `model` option is an ordinary entry
  (`{ id = "model", value = "…" }`). No deprecation sugar; consumer shims
  fail their `ptah check` gate on bump with the migration spelled out in the
  docs.
- **BREAKING**: `std/predicate`'s `PredicateOptions.model` is removed,
  replaced by `sessionConfig: {Entry}?` applied to every judge attempt
  session.
- New std module `std/session-config.luau` exports the `Entry` type and the
  shared `apply(session, entries?)` — the single home for apply-in-order
  semantics, required by both components and predicate so they cannot drift.
- No extra validation: `nil`/empty is a no-op; duplicate ids apply verbatim
  in order (last wins), exactly as repeated runtime `setConfig` calls would.
  Agent rejection fails through the existing `setConfig` error path; shape
  errors are caught by `ptah check` against the exported config types.
- Documentation rewritten to the new surface with a one-line migration note;
  `CONTEXT.md` gains the **Session config** vocabulary entry.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `factory-components`: the openspec and pr-review-loop component
  requirements replace `model`/`judgeModel` with ordered `sessionConfig`/
  `judgeSessionConfig` entries applied to every session each role creates
  (openspec's archive session included); the typed judge requirement's
  options replace `model` with `sessionConfig`; the component facade
  contract admits ordered entry arrays as data config.

## Impact

- `factory-components/std/session-config.luau` (new), `std/predicate.luau`
  (options surface), `components/openspec/component.luau`,
  `components/pr-review-loop/component.luau` (config types + application).
- `crates/ptah-cli/tests/factory_components.rs`: component shims rewritten to
  the new surface; new scenarios pinning ordered propagation to work, judge,
  human-probe, and archive sessions. Mock agent may gain observability for
  applied config (it already seeds `MOCK_CONFIG_OPTIONS` and tracks
  per-session state).
- `factory-components/README.md`, both component READMEs, `CONTEXT.md`.
- Consumer shims (source-mounted, flake-pinned): breaking on bump, caught by
  `ptah check` — the intended compatibility gate. No core crate, CLI, or
  type-definitions change.
