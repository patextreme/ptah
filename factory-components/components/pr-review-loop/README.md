# pr-review-loop component

Run a review→fix→push convergence against a pull request: an agent
reviews the PR following a reviewer instruction — configured text when
one is supplied, otherwise the component's built-in default — a typed
judge decides whether blocking issues remain, and the loop escalates to
a human, iterates through fixes and pushes, or converges and posts the
verdict as a PR comment. Extracted and generalized from identus-ws's
pr-review-loop.

## The instruction contract

The loop's convergence gate is a typed judge that asks whether the
review verdict contains **blocking issues**, and its fix prompts speak
the same vocabulary. That taxonomy is the contract between the loop
and its reviewer instruction:

- a configured reviewer instruction (`reviewInstruction`) must define
  what counts as a **blocking** issue for this repository and instruct
  the reviewer to classify findings as blocking or non-blocking;
- the loop's review prompt asks for that classification whether the
  instruction is configured or built in — enforcement for
  instructions that under-specify the output format (redundant with a
  compliant instruction by design).

Verdicts that do not reduce to a blocking/non-blocking classification
— score gates, approve/request-changes votes, report-only reviews —
are a **different component**, not an instruction swap: loop shape is
component policy, and this loop's shape is the convergence gate
above. Swapping the instruction changes what "blocking" means for the
repo, not what the loop does with the classification.

## The built-in default

Leave `reviewInstruction` nil and reviews run against the component's
built-in default instruction — no instruction to author, no
dependency on this repository's layout. The default
(`default-instruction.luau`, next to this README) is a full reviewer
persona that satisfies the contract: its Output section directs the
reviewer to classify every finding as BLOCKING (must be resolved
before the change is accepted) or NON-BLOCKING. It is the contract's
reference instance — copy it as the worked example when graduating to
a configured instruction.

A configured reviewer instruction is a full replacement, not a delta:
only a nil `reviewInstruction` selects the built-in default (an empty
string stays configured — a loud misconfiguration, not a silent
fallback). The default is normal versioned behavior: tuning it
changes zero-config reviews on upgrade, and configuring an
instruction is how a repo pins its reviewer.

## The pointer pattern

A long or repo-pinned reviewer instruction is best supplied as text
that references a versioned repository document — one shim line
pointing at the file — rather than inlining the document's content:

```lua
reviewInstruction = "Follow the reviewer instruction at .ptah/instructions/reviewer.md",
```

The component treats such text identically to any other (pointer-style
text cannot be reliably detected, so it is never special-cased). The
honest trade: configured text is inlined into every iteration's review
prompt (up to `maxIterations` per run) — fine at the built-in
default's size, and the pointer pattern is the escape hatch for
longer instructions.

## Environment requirements (declared, not bundled)

- **Work agent able to act on the repository and the PR host** — read
  the repo, push commits to the PR branch, and comment on the PR
  (typically via the `gh` CLI on the agent's PATH, with credentials in
  the agent subprocess's environment).
- **Any document your reviewer instruction references** — must exist
  and be readable by the agent (only pointer-style instructions
  reference one; the built-in default requires nothing).
- **Judge agent** — any agent that can answer typed boolean prompts.

## Config (data plus declared agent handles)

```lua
local prReview = require("<mount>/factory-components/components/pr-review-loop/component")

local loop = prReview.new({
	agent = ptah.agent("claude"),        -- work agent handle
	judgeAgent = ptah.agent("claude"),   -- judge agent handle
	model = "opus",                      -- optional: work model id
	judgeModel = "haiku",                -- optional: judge model id
	-- optional (default when nil: the built-in instruction): the
	-- entire reviewer instruction as text; must classify findings
	-- blocking/non-blocking (the instruction contract above). Long or
	-- repo-pinned instructions usually take the pointer pattern —
	-- text referencing a versioned document:
	-- reviewInstruction = "Follow the reviewer instruction at .ptah/instructions/reviewer.md",
	dryRun = false,                      -- optional: never push (default false)
	maxIterations = 15,                  -- optional: cap (default 15)
})
```

## Operations

- `loop:review(prUrl)` — run the loop against one pull request; the PR
  URL is per-call data and the sole repository context. Returns the
  final accepted review verdict text.

With `dryRun = true` the commit-and-push step is skipped entirely: the
loop still reviews, judges, and fixes, but never pushes to the PR
branch — a gate for rehearsing instruction changes against a real
reviewer without pushing. The converged session still posts the
verdict comment: dry-run gates the branch, not the PR conversation.

The component ships facade-only (`:review`). A `run()` daemon
convenience (looping over open PRs) was deliberately deferred: it is
sugar over `std.daemon` + `:review` and can be added without breaking
the facade.
