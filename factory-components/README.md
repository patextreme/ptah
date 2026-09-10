# Factory Components

The shared workflow library: repo-agnostic helper modules (`std/`) and
composable workflow components (`components/`) that consumer repositories
mount as source and drive through thin shims. See the archived
`factory-components` change
(`openspec/changes/archive/2026-09-04-factory-components/`) for the
distribution decision and `CONTEXT.md` for the vocabulary.

## Layout

- `std/` — the stdlib: repo-agnostic machinery that knows nothing about
  any consumer repo.
  - `session-config.luau` — ordered session-config entries applied to a
    session as `setConfig` calls (the shared apply mechanism; see
    [Session config](#session-config)).
  - `predicate.luau` — typed boolean judge (asks an agent whether a
    predicate holds for a payload; bounded retry, exhaustion is a script
    error).
  - `gh.luau` — GitHub CLI transport over `ptah.exec` with structured
    outcomes (never raises for a failed command) and POSIX-safe argument
    quoting.
  - `daemon.luau` — repo loop skeleton: apply a per-repo operation with
    per-repo error isolation, sequential or bounded-concurrency parallel.
- `components/<name>/` — one directory per component: `component.luau`
  is the facade module exposing `new(config) -> instance`, and the
  sibling `README.md` declares the component's environment
  requirements. (The module file is `component.luau` — not
  `init.luau` — so ptah's require resolver and luau-lsp's agree on the
  module's internal `../../std/…` requires; an `init.luau` module's
  relative requires resolve one directory off under luau-lsp.)
  - `openspec/` — groom, implement, and verify an openspec change.
  - `pr-review-loop/` — review→fix→push convergence against a pull
    request.

## Loop conventions

std ships only mechanisms a third consumer would use verbatim
(transport, typed verdicts, capped retry, isolation) — loop *shape* is
component policy, so each component writes its convergence loop over
`std/predicate` in exactly the shape its workflow needs. The components'
loops share these conventions, documented here so drift stays visible:

- Sessions: per-iteration work sessions are `<prefix>:<n>`, judge
  sessions `<prefix>-judge:<n>`, and escalation-judge sessions
  `<prefix>-human:<n>`.
- Every prompt of a loop is prefixed `[<prefix> iteration N of M]` so
  the agent (and the logs) can see the loop state.
- Failure wording: `<prefix>: human input is required to resolve the
  findings (iteration N of M)` and `<prefix>: did not converge within M
  iterations`.

## Session config

A session-config entry is `{ id: string, value: string | boolean }` —
one `session:setConfig(id, value)` call. A session-config field
(`sessionConfig`, `judgeSessionConfig`, or `std/predicate`'s
`sessionConfig`) takes an ordered *array* of entries, and the array is
the consumer's `setConfig` call sequence as data: entries apply in
declared order after session creation, before the session's first
prompt, via the shared `std/session-config.apply` — the components and
the judge call it so application semantics cannot drift.

- **Order is load-bearing.** Agents with dependent options (opencode
  re-derives `effort` from every `model` set) only behave when the
  driving option is set first — which is why the field is an array, not
  a table.
- **No extra validation.** `nil`/empty applies nothing; duplicate ids
  apply verbatim in order (last wins, exactly as repeated runtime
  `setConfig` calls); an agent-rejected entry fails through the
  existing `setConfig` error path with entries before it left applied.
  Option ids and values are the agent's authority — enumerate them with
  `session:configOptions()`.
- **Mixed string/boolean values in one literal** may need an explicit
  annotation (`local entries: { sessionConfig.Entry } = …`) — the
  analyzer infers one element type per unannotated array literal.

**Migrating from `model`/`judgeModel`** (removed in the same change that
introduced entries — a model choice is an ordinary entry):

```lua
-- before
model = "claude-opus-4-5",
judgeModel = "claude-haiku-4-5",
-- after
sessionConfig = { { id = "model", value = "claude-opus-4-5" } },
judgeSessionConfig = { { id = "model", value = "claude-haiku-4-5" } },
```

`ptah check` names a removed field when you bump — that is the
compatibility gate doing its job.

## The component contract

- A component is a facade of typed operations: `new(config)` returns an
  instance; instance methods are the operations; per-call data (a change
  name, a PR URL) is a method argument, not config.
- Config is data — strings, numbers, booleans — plus ptah runtime
  handles where the component's config type declares them
  (`agent: Agent`). Functions are not configuration.
- Every module is `--!strict` and every component exports its `Config`
  type, so a consumer's `ptah check` validates their config against the
  component's config type (the compatibility gate).
- Library modules only require other modules inside this tree, never
  write files inside it, and never depend on a relative working
  directory — so the tree works mounted anywhere (including a read-only
  nix store path).

## Consuming

Mount the tree wherever you like (nix flake input + symlink, submodule,
vendored copy), then write a shim — the only workflow code you own:

```lua
--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")

local ops = openspec.new({
	agent = ptah.agent("claude"),
	judgeAgent = ptah.agent("claude"),
	sessionConfig = { { id = "model", value = "claude-opus-4-5" } },
	judgeSessionConfig = { { id = "model", value = "claude-haiku-4-5" } },
})

ops:groom("add-auth")
```

`require` paths are relative to the requiring file and may traverse
outside the shim's directory, so the mount point is your free choice.
Pin the mount with whatever mechanism mounted it (`flake.lock`, submodule
ref); `ptah check` on your shim is the compatibility gate when you bump.
