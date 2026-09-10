## Context

The ptah core deliberately has no constructor config table: `pairs()`
iteration order is unspecified and order is load-bearing for agents with
dependent options (opencode re-derives `effort` from every `model` set), so
`session:setConfig(...)` after creation — in the order the script writes the
calls — is the creation-time configuration path (see the ptah README's
session-config section).

The factory-components library has the opposite problem: components create
their own sessions internally, and the component contract forbids callable
hooks, so a consumer cannot make those `setConfig` calls themselves. Today
the library's only configuration lever is the `model`/`judgeModel` string on
both components and `model` on `std/predicate`'s `PredicateOptions` — one
hard-coded option id, applied first by construction, unable to express
dependent options at all.

## Goals / Non-Goals

**Goals:**

- Let consumers express the full `setConfig` sequence for work and judge
  sessions as data, with application order explicit.
- One shared implementation of apply-in-order semantics — components and
  judge cannot drift.
- A single coherent config surface: one concept (`sessionConfig` entries)
  replaces the special-cased `model` fields everywhere it existed.

**Non-Goals:**

- Any change to the ptah core runtime, CLI, or type definitions —
  `setConfig`/`configOptions` semantics are untouched.
- Config-option discovery or validation beyond the agent's own authority:
  the library never inspects ids or values; a rejected entry fails through
  the existing error path.
- Deprecation shims or parallel surfaces — the removal is a clean cut;
  pinned consumers migrate at bump time via `ptah check`.

## Decisions

### D1: Ordered entry array, not a table

`sessionConfig = { { id = "model", value = "opus" }, { id = "effort", value = "high" } }`

The entry mirrors `setConfig(id, value)`'s parameters 1:1; the array *is*
the call sequence as data. This is the design point where the library
differs from the core's "no config tables" rule — and how it stays honest to
it: the core rejects tables precisely because a table cannot carry order;
an array can, so the consumer's intent (an ordered `setConfig` sequence)
survives being expressed as data.

Alternatives:
- *Plain table with a "model first" convention* (the issue's original
  proposal) — cheaper to write, but silently lossy for any dependency other
  than model→X, and every unwritten rule (model without table, `model` key
  inside the table, iteration determinism) becomes a spec clause. Rejected.
- *Keep `model` as sugar over the array* — reintroduces two sources for one
  option and a precedence rule between them. Rejected.

### D2: `model`/`judgeModel` removed, not deprecated

The `model` option becomes an ordinary entry (`{ id = "model", value = "…" }`),
which is what ACP says it is: an option the agent defines. The fields are
removed from both components' `Config` types and from `PredicateOptions` in
the same change, with no parallel surface. Consequences accepted:
source-mounted consumers (identus-ws pinned at `87454e1`, the merge-bot
consumer) break on their next bump, and `ptah check` names the removed
field — the documented compatibility gate. The repo's own component tests
and READMEs migrate in the same change.

Precedent: the drop-converge-loop change deleted `std/converge.luau` with
"no replacement module, no deprecation shim" for the same reason. One
caveat that change could lean on has aged: "external consumers are
components-only" no longer holds (the merge-bot consumer calls `std/gh` and
`std/daemon` directly), so the `PredicateOptions` break is more visible —
hence the explicit migration note in the READMEs rather than relying on the
change archive alone.

### D3: One std module owns the semantics

`factory-components/std/session-config.luau` exports the `Entry` type and
`apply(session, entries?)`. The components call it for work sessions
(openspec: per-iteration + archiver; pr-review-loop: per-iteration), and
`std/predicate` calls it for judge attempt sessions. The module is the
third std consumer alongside two components — enough usage evidence for the
"mechanism a third consumer would use verbatim" bar, and any consumer
writing a custom component with config-declared options requires it
directly.

Alternative: duplicating ~8 lines per component — cheap until the semantics
get a second clause, then three copies drift. Rejected (the issue itself
asks for shared logic).

### D4: No validation beyond the type gate and the agent

- `nil`/empty array → no `setConfig` calls at all.
- Duplicate ids → applied verbatim, last wins (runtime `setConfig` permits
  repeated calls; the library does not invent a stricter contract).
- Wrong-typed values, missing `id` → `ptah check` findings against the
  exported `Config` type (strict-mode table types), never runtime errors.
- Unknown id / rejected value → the agent rejects; the existing
  `setConfig` error path raises with the option id and agent message.

Adding a library-side duplicate-key or unknown-id check would create a
second validation contract diverging from both the core runtime and the
agent's authority. Rejected.

### D5: Test observability via the mock's echo

The mock agent seeds config options (`MOCK_CONFIG_OPTIONS`), tracks live
per-session state through `session/set_config_option`, and already obeys a
re-derivation contract (`model` re-derives `effort`). Propagation tests
assert on config state the mock echoes into prompt responses — the same
mechanism the component tests already use (the mock echoes prompts back
into judge payloads). If the echo does not carry config state today, extend
the mock (the sanctioned path for new agent behavior in tests) rather than
asserting on stderr side channels.

## Risks / Trade-offs

- [Breaking bump for pinned consumers] → Deliberate (D2). The READMEs carry
  a one-line migration (`model = "x"` → `sessionConfig = { { id = "model",
  value = "x" } }`); `ptah check` names the removed field at bump time.
- [Entry-array ceremony for the common model-only case] → Accepted: one
  extra line per model pin buys order-expressiveness and a single concept;
  the README examples show the idiom once and it reads clearly.
- [Luau excess-property checking gaps: does `model = …` reliably produce a
  check finding?] → Observed: the analyzer accepts unknown keys in every
  construction shape, so Config declares the removed fields as nil-typed
  tombstones (`model: nil`) — any configured value is a type error
  naming the field, while absence stays clean. Pinned by the
  compatibility-gate scenario; the READMEs document the migration to the
  entry form.

## Migration Plan

Single change lands the new surface and removes the old one; there is no
window where both exist. Repo-internal migration (tests, READMEs,
`.ptah/` shims if any use the fields) happens in the same change. Rollback
is reverting the change; consumers who never bump are never affected.
