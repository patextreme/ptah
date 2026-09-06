# openspec component

Groom, implement, and verify a named openspec change through a
convergence loop: prompt an agent (running the openspec skills), judge
the output with a typed predicate, fix and repeat until the predicate
holds, a human is needed, or the iteration cap is reached.

## Environment requirements (declared, not bundled)

This component drives an agent through the openspec workflow skills and
the `openspec` CLI. Before running, verify the environment provides:

- **Work agent carrying the openspec skills** — the agent you pass
  as `agent` (a handle you construct, e.g. from a registry name or an
  inline agent spec) must have the openspec skill set installed
  (`openspec-review`, `openspec-apply-change`,
  `openspec-verify-change`, and the archive lifecycle) and must be able
  to read and write your repository.
- **`openspec` on PATH** — the agent invokes the `openspec` CLI to read
  change state and to sync/archive; it must be resolvable in the
  environment the agent's subprocess inherits.
- **Judge agent** — any agent that can answer typed boolean prompts
  (a small/fast model is ideal); it needs no openspec skills.

The component installs none of these itself.

## Config (data plus declared agent handles)

```lua
local openspec = require("<mount>/factory-components/components/openspec/component")

local ops = openspec.new({
	agent = ptah.agent("claude"),       -- work agent handle
	judgeAgent = ptah.agent("claude"),  -- judge agent handle
	model = "opus",         -- optional: model config id for work sessions
	judgeModel = "haiku",   -- optional: model config id for judge sessions
	maxIterations = 10,     -- optional: convergence cap (default 10)
})
```

Functions are not configuration; every field is data or a declared
agent handle.

## Operations

Per-call data is a method argument:

- `ops:groom(change)` — converge the change's proposal through review
  (`openspec-review`); exits when the review carries no blocker
  findings, escalating to a human or the cap otherwise.
- `ops:implement(change)` — drive task execution
  (`openspec-apply-change`) until all tasks of the change are
  implemented. Each pass ends either complete or paused with a stated
  reason, as the skill defines those states: a pause the agent can
  resolve itself (e.g. updating the change's artifacts) is resolved and
  the loop continues; a pause that needs human input fails the
  operation.
- `ops:verify(change)` — converge verification
  (`openspec-verify-change`) until it reports no critical findings or
  warnings, then sync and archive the change in the same operation.

Each operation returns the final accepted review text.
