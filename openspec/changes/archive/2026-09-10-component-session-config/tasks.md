## 1. std/session-config — the shared mechanism

- [x] 1.1 Create `factory-components/std/session-config.luau` (`--!strict`):
      export `Entry = { id: string, value: string | boolean }` and
      `apply(session: Session, entries: { Entry }?)` that issues one
      `setConfig(entry.id, entry.value)` per entry in array order, applying
      nothing for nil/empty; verify with a ptah-run script against the mock
      (seeded `MOCK_CONFIG_OPTIONS`, order-sensitive assertions via the
      mock's config-state echo; extend the mock's echo if it does not carry
      config state today) and `ptah check` on the script
- [x] 1.2 Add std scenarios to
      `crates/ptah-cli/tests/factory_components.rs`: entries apply in
      declared order (model then effort, relying on the mock's re-derive
      contract), nil/empty is a no-op, duplicate ids apply verbatim with
      last-wins, agent rejection raises the `setConfig` error carrying the
      option id; verify with `cargo test --test factory_components session_config`

## 2. std/predicate — judge surface

- [x] 2.1 Replace `PredicateOptions.model` with
      `sessionConfig: { sessionConfig.Entry }?`, applied through
      `session-config.apply` to every judge attempt session; verify the
      existing predicate tests still pass after migrating their opts and
      `cargo test --test factory_components` is green
- [x] 2.2 Add a judge-propagation scenario: entries configured on a
      predicate call reach the judge session in declared order before the
      predicate prompt; verify with `cargo test --test factory_components predicate`

## 3. Components — config surface and application

- [x] 3.1 `factory-components/components/openspec/component.luau`: replace
      `model`/`judgeModel` with `sessionConfig`/`judgeSessionConfig`
      (`{ sessionConfig.Entry }?`), apply work entries to every
      per-iteration work session and the verify archive session, forward
      judge entries into every `predicate(...)` call (judge + human
      probes); verify `ptah check factory-components/components/openspec/component.luau`
- [x] 3.2 Same for
      `factory-components/components/pr-review-loop/component.luau`: work
      entries on every per-iteration session (which also posts the verdict
      comment), judge entries on judge + human-probe calls; verify
      `ptah check factory-components/components/pr-review-loop/component.luau`
- [x] 3.3 Migrate the existing component test shims in
      `crates/ptah-cli/tests/factory_components.rs` from `model`/`judgeModel`
      to entry arrays (`converges_on_second_pass`, openspec/pr-review-loop
      shims, the gate-clean shim); verify
      `cargo test --test factory_components` is green
- [x] 3.4 Add component scenarios pinning propagation: openspec work +
      archive sessions receive `sessionConfig`, judge/human-probe sessions
      receive `judgeSessionConfig`, and the same pair for pr-review-loop
      work sessions; verify each new test passes against the mock offline
- [x] 3.5 Add compatibility-gate scenarios (extend
      `mistyped_component_config_is_a_check_finding` or sibling): a shim
      passing the removed `model`/`judgeModel` is a `ptah check` finding
      naming the field, and a wrong-typed entry value
      (`value = 42`, missing `id`) is a finding naming the entry shape;
      verify with `cargo test --test factory_components` inside
      `nix develop` (real analyzer)

## 4. Dogfood shims

- [x] 4.1 Migrate `.ptah/workflows/openspec/main.luau`,
      `.ptah/workflows/pr-review-loop/main.luau`, and
      `.ptah/workflows/adhoc/main.luau` from `model`/`judgeModel` to
      session-config entries (work `zai/glm-5.3`, judge
      `zai/glm-5.3-flash`); verify `ptah check` passes on each migrated
      shim

## 5. Documentation and vocabulary

- [x] 5.1 `factory-components/README.md`: add `session-config` to the std
      list, document the entry form (order = the consumer's `setConfig`
      order, duplicates verbatim, no extra validation, agent authoritative)
      with the migration note for `model`/`judgeModel`, and rewrite the
      consuming example to the new surface
- [x] 5.2 Both component READMEs: document `sessionConfig`/
      `judgeSessionConfig` (which sessions each reaches, ids are
      agent-specific — enumerate `configOptions()`), replace model-field
      examples with entry examples; verify `stylua .` clean on all edited
      `.luau` and docs mention no removed field except in the migration note
- [x] 5.3 `CONTEXT.md`: add the **Session config** entry (the ordered list
      of `(id, value)` entries a component applies to every session it
      creates, via `setConfig`, in declared order — the consumer's
      `setConfig` sequence as data) with avoids (`model config` — model is
      one entry, not the concept; `config table` — a table cannot carry
      order); verify it uses the CONTEXT.md format

## 6. Gates

- [x] 6.1 `cargo test` (full suite) green inside `nix develop`
- [x] 6.2 `stylua .` clean and `ptah check` green on every edited `.luau`
      file (library modules, component entries, dogfood shims)
