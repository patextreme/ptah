---
name: ptah
description: Author, validate, and run Luau automation scripts for ptah — the CLI that drives ACP-speaking AI agents (Claude Code, Gemini CLI, Codex, …) headlessly from sandboxed Luau. Use this skill whenever you are writing or editing a .luau script that uses the `ptah` global (ptah.agent, session:prompt, ptah.parallel, ptah.spawn, ptah.exec, ptah.ask, ptah.json, resultSchema, configOptions, setConfig), configuring the agent registry (.ptah/config.toml), running or debugging `ptah run` / `ptah check` / `ptah types`, or whenever the user wants to orchestrate, fan out, pipeline, or watchdog multiple AI agents as code, run deterministic shell steps between agent turns, or pause a run to ask a human (ptah.ask, --ask, PTAH_ASK, the [ask] registry section). Also trigger when debugging ptah script errors, exit codes, timeouts, cancels, typed results, or concurrency behavior.
---

# ptah scripting

`ptah run script.luau` executes a Luau script that drives any
ACP-speaking agent over stdio. Scripts read as plain synchronous code —
`reply = session:prompt("…")` blocks the script, not the runtime — while
the runtime multiplexes subprocesses underneath. Each `agent:session()`
owns its own agent subprocess; closing a session reaps the process.

- Source repo (deep investigation): **https://github.com/patextreme/ptah**
  - `README.md` — the full API/behavior truth. Read it first when this
    skill's summary is not enough.
  - `.ptah/ptah.d.luau` — the script API type definitions, embedded
    verbatim in the binary; the single source of truth for shapes.
  - `examples/*.luau` — vetted scripts (sequential, fan-out, model
    fan-out, watchdog, typed results, exec pipeline).
  - `crates/ptah-luau/` (Luau runtime + sandbox), `crates/ptah-acp/`
    (ACP client), `crates/ptah-check/` (`check` pipeline),
    `crates/ptah-cli/src/bin/mock-agent/` (scriptable test agent used
    by the offline test suite).
- `ptah types` prints the type definitions matching the *installed*
  binary — no registry or agents needed. `ptah --version` confirms the
  binary exists.

## Workflow

1. **Confirm the toolchain**: `ptah --version`. No binary → build from
   the repo (`nix build`, or `cargo build` in its dev shell).
2. **Discover available agents**: read `.ptah/config.toml` walking up
   from the directory `ptah` will be invoked in, then
   `~/.config/ptah/config.toml`. Project entries override user entries
   per agent *name*. If no registry fits, either write one or pass an
   inline spec (see [Registry](#registry)).
3. **Write the script**: first line `--!strict`. Sandbox rules below.
4. **Validate without executing**: `ptah check script.luau`. Fix every
   finding (details below). Exit `2` means the check itself could not run
   (missing script, registry discovery failure, or no `luau-lsp` on PATH) —
   fix that environment problem first.
5. **Run**: `ptah run script.luau`. Diagnose with `--verbose`
   (lifecycle), `-vv` (also forwards agent subprocess stderr),
   `--no-color` (plain text), `--quiet` (suppress rendering; script
   `print` still passes through).
6. Optional, when luau-lsp is available for editor/type support: run
   `ptah init` — it writes `.ptah/ptah.d.luau` (definitions matching
   the installed binary) and a commented `.ptah/config.toml` skeleton
   in `./.ptah/`, skipping files that already exist — then configure
   luau-lsp (platform `standard`) to use `.ptah/ptah.d.luau`.
   Definitions are workspace-wide — keep them out of mixed Luau
   projects not run under ptah. After upgrading ptah, refresh with
   `ptah types > .ptah/ptah.d.luau`.

## Script rules (the sandbox)

Scripts run in a curated Luau environment. Available: `string`, `table`,
`math`, `utf8`, `bit32`, `buffer`, `os.time`, `os.clock`, `os.getenv`, `print`, the
standard Luau base library (`pcall`, `error`, `assert`, `tostring`,
`tonumber`, `pairs`, `ipairs`, `select`, `type`/`typeof`,
`setmetatable`/`getmetatable`, `rawget`/`rawset`, …), and a restricted
`coroutine` table with only `yield` (the async runtime needs it). There is
**no** file I/O, no `io`, no network, no
`debug`, no `loadstring`/`collectgarbage`, and the ambient globals expose
no subprocess execution — world access arrives through injected
capabilities, and `ptah.exec` (see below) is the shell door, always
injected by the `ptah` CLI. Orchestration logic belongs in the script;
machine work belongs in `ptah.exec`, the *agent's* prompt, or the agent
registry.

`require` resolves `.luau` modules relative to the requiring file
(`foo.luau`, `foo.lua`, `foo/init.luau`, `foo/init.lua`) with no directory
boundary — `require("../shared/helper")` reaches sibling trees — and
rejects non-relative require strings (absolute paths, bare module names,
aliases). Scripts are trusted code: they drive agents with the user's full
authority, and the sandbox limits the blast radius of bugs, not malice.
`--!strict` is enforced by `ptah check` on the entry and every reachable
file.

## API reference

### `ptah` namespace

| API | Description |
| --- | --- |
| `ptah.agent(name_or_spec)` | Agent factory. `name` resolves against the registry (raises `unknown agent \`name\`…` at this call when missing); inline spec `{ command =, args = {…}, env = {…} }` skips the registry. `${VAR}` in inline `env` values interpolates from ptah's environment. |
| `agent:session(opts?)` | New session (own subprocess). Returns an `Agent`-scoped handle; `opts` below. |
| `session:prompt(text, { timeoutMs = n }?)` | One turn → `PromptResult` (below). **Timeout raises a catchable Lua error** after sending a cancel — `pcall` it if you need to survive. |
| `session:cancel()` | Cancel the in-flight turn; the blocked `prompt` returns normally with `stopReason = "cancelled"`. |
| `session:close()` | End session, reap the process. |
| `session:label()` | `"agentName/sessionId"` string (a method — call with `:`). Handy for logs. |
| `session:configOptions()` | Live per-session config options (see [Config](#per-session-config-models-etc)). |
| `session:setConfig(id, value)` | Set a config option between turns; raises a catchable error carrying the id + agent message on rejection. |
| `ptah.spawn(fn)` → `task:await()` | Concurrent task; errors re-raise at the await site. |
| `ptah.join({task, …})` | Wait for tasks → per-task outcome entries. |
| `ptah.parallel(items, fn, { concurrency = n }?)` | Fan-out (default unlimited) → per-item outcome entries in item order. |
| `ptah.exec(cmd, { timeoutMs = n }?)` | Run a shell command via `/bin/sh -c` → `{ exitCode, stdout, stderr }`. Any exit code is data; only could-not-run and timeout raise (process group killed first). See [Shell exec](#shell-exec-ptah-exec). |
| `ptah.ask({ prompt =, details = })` | Pause for a human answer → `{ action = "respond", text = … }` or `{ action = "abort" }`; suspends only the calling coroutine. See [Asking a human](#asking-a-human-ptahask). |
| `ptah.json.parse(s)` / `ptah.json.stringify(v, { indent = n }?)` | Pure JSON decode (`null` → `nil`, raises on malformed input) / encode (string keys only, arrays are 1..n). |
| `ptah.sleep(ms)` / `ptah.log(msg)` / `ptah.exit(code)` / `ptah.version` | Helpers. `log` renders with a `[ptah]` prefix; `exit` terminates the process with code `n`. |

Session options (`agent:session({...})`; all optional):

- `id` — session label; defaults to `s1, s2, …` per agent. `"exec"` is
  reserved (exec lifecycle attribution) and rejected at session
  creation.
- `cwd` — working dir for the agent subprocess; defaults to the
  invocation directory.
- `mcpServers` — suggested MCP servers: `{ type = "stdio", name =,
  command =, args =, env = }` or `{ type = "http", name =, url =,
  headers = }`.
- `resultSchema` — typed-result contract (see below).

(No `config` option exists — it was removed. Passing `config = { … }`
raises a catchable error naming `setConfig` as the replacement, before
any agent subprocess spawns.)

`PromptResult`:

- `text` — the turn's **last agent message** (final contiguous text run;
  tool activity ends a run; falls back to the previous non-empty run if
  the turn ends on tool activity; `""` for cancelled turns). Narration
  streams to the terminal but does not pollute `r.text`. `tostring(r)`
  is `r.text` at runtime and type-checks fine (`tostring` takes `any`);
  the sugar itself is not covered by the type definitions, so write
  `r.text` explicitly wherever the checker expects a `string` — implicit
  coercion (`r .. ""`, a `string`-typed binding) is a type error.
- `stopReason` — typed plain `string` (the checker won't enforce
  exhaustiveness), but a script only ever observes these five values:
  `"end_turn"`, `"max_tokens"`, `"max_turn_requests"`, `"refusal"`,
  `"cancelled"`. ptah normalizes any unrecognized reason to
  `"end_turn"` at the ACP boundary.
- `usage` — `{ input, cacheRead, cacheWrite, output }` (zeros when
  unreported).
- `result` — the turn's typed submission, or `nil` (see below).

## Semantics that bite

- **One turn at a time per session.** Concurrent `prompt` calls on the
  same session serialize behind a turn lock — they queue, they do not
  overlap. For real parallel turns, create one session per concurrent
  lane (see the pool pattern below).
- **Headless permissions.** Nobody is there to answer prompts, so ptah
  auto-allows every permission request (first `AllowAlways`, else the
  first allow option). Everything else an agent may ask of a client gets
  method-not-found — turns never hang. Note `AllowAlways` can persist an
  allow rule in the agent's own config beyond the run.
- **Asking a human is a separate channel.** `ptah.ask` pauses only the
  calling coroutine (sessions and other tasks keep running; a pending ask
  keeps the run alive). The provider is an operator decision — `--ask` >
  `PTAH_ASK` > project `[ask]` > user `[ask]` > auto-detect (TTY-only) —
  and `none` prohibits asking without touching the headless permission
  posture above. Abort is data, not an error; the four failures
  (prohibited, no provider, provider failure, stdin EOF) raise distinct
  errors.
- **Timeout vs cancel.** `timeoutMs` expiry sends a cancel, then *raises*
  a Lua error. `session:cancel()` makes the prompt *return* with
  `stopReason = "cancelled"` (`text` is `""`, `result` discarded).
- **Exit codes are a contract.** `0` success; `1` uncaught script error,
  a never-observed task error (a failed task nobody awaited), or `check`
  findings; `2` CLI usage errors (and "check could not run"); `n` when
  the script calls `ptah.exit(n)`; `130`/`143` when the run is cancelled
  by SIGINT/SIGTERM (teardown — killing in-flight execs and sessions —
  runs before the exit; a second signal exits immediately).
- **Un-awaited task errors still fail the run** (exit 1, reported to
  stderr). Always `await()`/`join()` your tasks, or handle their outcome
  entries.

## Factory Components (the shared workflow library)

Before hand-rolling a review loop, a typed judge, a `gh` transport, or
a repo fan-out daemon, check the bundled library under
`factory-components/` (in the ptah repo; consumer repos mount it as
source — nix flake input + symlink, submodule, or vendored copy):

- `std/predicate` — typed boolean judge (bounded retry; no verdict is a
  script error, never a hang)
- `std/gh` — GitHub CLI transport over `ptah.exec` (structured
  outcomes, never raises for a failed command, POSIX-safe quoting)
- `std/daemon` — per-repo loop with error isolation (sequential or
  bounded parallel)
- `components/openspec` — groom/implement/verify an openspec change
- `components/pr-review-loop` — review→fix→push convergence on a PR

Consumption is a **shim** — the only workflow code the consumer repo
owns:

```lua
--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")

local ops = openspec.new({
	agent = ptah.agent("claude"),       -- work agent handle
	judgeAgent = ptah.agent("claude"),  -- judge agent handle
	sessionConfig = { { id = "model", value = "claude-opus-4-5" } },
	judgeSessionConfig = { { id = "model", value = "claude-haiku-4-5" } },
})

ops:groom("add-auth")
```

Rules of the contract: mount the tree anywhere (requires are relative
and the library never requires out of its tree, so the mount point is
free — a read-only store path works); config is data plus declared
ptah runtime handles where the component's config type declares them
(`agent: Agent`; functions are not configuration) and per-call data
(change name, PR URL) is a method argument; `ptah check` on the shim is the compatibility gate —
component `Config` types are strict, so a mistyped field is a type
error naming the field. See the ptah repo's `factory-components/`
README and each component's README for environment requirements.

## Patterns

Sequential turns (state accumulates across prompts in one session):

```lua
--!strict
local agent = ptah.agent("claude")
local s = agent:session({ id = "reviewer" })
for _, file in ipairs({ "src/a.rs", "src/b.rs" }) do
    local r = s:prompt(("Review %s for obvious bugs; be terse."):format(file))
    ptah.log(("%s -> %s"):format(file, r.text))
end
s:close()
```

Parallel fan-out with a session pool (concurrency cap = pool size; one
session per lane so turns genuinely overlap):

```lua
--!strict
local agent = ptah.agent("claude")
local targets = { "alpha", "beta", "gamma", "delta", "epsilon" }

local outcomes = ptah.parallel(
    targets,
    function(target: string)
        -- one session per item: parallel turns, isolated context
        local s = agent:session({ id = "rev-" .. target })
        local r = s:prompt(("Summarize the changes in %s."):format(target))
        s:close()
        return ("%s: %s"):format(target, tostring(r))
    end,
    { concurrency = 2 }
)

for i, entry in ipairs(outcomes) do
    if entry.ok then
        ptah.log(entry.value)
    else
        ptah.log(("FAILED %d: %s"):format(i, entry.error))
    end
end
```

Typed results — declare a JSON Schema, then nil-check retry (degradation
is designed: agents may ignore the submit tool, sandboxes may block it;
the turn still completes normally with `result = nil`):

```lua
--!strict
local s = ptah.agent("claude"):session({
    id = "reviewer",
    resultSchema = {
        type = "object",
        properties = {
            verdict = { type = "string", enum = { "approve", "block" } },
            score = { type = "integer", minimum = 0, maximum = 10 },
        },
        required = { "verdict", "score" },
    },
})

for attempt = 1, 3 do
    local r = s:prompt("Review the diff; be terse.")
    if r.result ~= nil then
        ptah.log(("%s (%d/10)"):format(r.result.verdict, r.result.score))
        break
    end
    ptah.log("no typed result; retrying")
end
s:close()
```

Watchdog cancel (a slow turn returns `stopReason = "cancelled"`):

```lua
--!strict
local agent = ptah.agent("claude")
local s = agent:session({ id = "slow" })

local work = ptah.spawn(function()
    return s:prompt("Take your time on this one").stopReason
end)
ptah.sleep(30_000)
s:cancel()
assert(work:await() == "cancelled")
s:close()
```

## Shell exec (`ptah.exec`)

Between agent turns there is deterministic work — list PRs, run a
build, format files. `ptah.exec(cmd, opts)` runs it inside the script
and hands the result back as data, so deterministic steps and agent
turns compose:

```lua
--!strict
local r = ptah.exec("gh pr list --json number,title --limit 20")
if r.exitCode ~= 0 then error("gh failed: " .. r.stderr, 0) end
for _, pr in ipairs(ptah.json.parse(r.stdout)) do
    ptah.log(("#%d %s"):format(pr.number, pr.title))
end
```

- **Invocation** is one string through `/bin/sh -c` — pipelines,
  redirections, `&&` work. POSIX `sh` semantics only: bashisms
  (`[[ ]]`, arrays, `$'…'`) are not guaranteed (`/bin/sh` is dash on
  Debian-family systems). With dynamic data, single-quote arguments and
  escape embedded single quotes as `'\''` — there is no portable
  `printf %q` under POSIX sh.
- **Any exit code is data**: the call returns
  `{ exitCode, stdout, stderr }` and never raises for a failed command
  (a signal death maps to the shell's `128 + signal`). Only two
  conditions raise a catchable Lua error: the command could not run at
  all, and `timeoutMs` elapsing — the timeout error names the command
  and the budget, and the process *group* is killed first (mirroring
  the prompt-timeout contract). No `timeoutMs` means no budget.
- **Blocking is per-coroutine**: the calling script blocks, but spawned
  tasks and other sessions keep progressing.
- **Environment and cwd are inherited**; stdin is closed (`/dev/null`),
  so a child that prompts fails fast on EOF. No `cwd`/`env` overrides
  in v1.
- **Output is captured, not streamed**: each exec renders one start
  line (the command) and one end line (exit code + duration) as
  `[ptah] exec: …`; `--quiet` suppresses them like all rendered
  output.
- **Teardown kills in-flight execs**: a script error, `ptah.exit`, or
  cancellation kills every in-flight process group — no orphans. A
  teardown-cancelled exec raises `exec \`cmd\` cancelled: the run is
  ending` inside its own coroutine; it never changes the run's own
  exit code (`ptah.exit(0)` with an exec in flight still exits 0).

The session id `exec` is reserved for these lifecycle lines —
`agent:session({ id = "exec" })` is rejected; choose another id.
`ptah.json.parse` / `stringify` exist for this pattern (pure, no I/O;
`null` → `nil`; string keys only; `{ indent = n }` pretty-prints).

## Asking a human (`ptah.ask`)

Workflows hit blockers only a human can resolve. Raising an error unwinds
the run and the session with it; `ptah.ask(opts)` pauses instead:

```lua
--!strict
local answer = ptah.ask({
	prompt = "Two candidates for the fix — which approach?",
	details = "a) retry with backoff  b) fail over to the replica",
})
if answer.action == "respond" then
	ptah.log("proceeding with: " .. answer.text)
else -- abort: handle it like any other value
	ptah.exit(1)
end
```

- **Opts**: `{ prompt = <string>, details = <string>? }` — `prompt`
  required (a usage error names it), `details` renders as one indented
  line. No timeout option exists: an ask blocks until answered, aborted,
  or the run is cancelled.
- **Result**: `{ action = "respond", text = … }` (the answer line,
  unprocessed) or `{ action = "abort" }` (no `text`). With the `stdin`
  provider, a line of exactly `/abort` — or Ctrl-D on an empty terminal
  prompt — aborts.
- **Distinct raises** (all `pcall`-able): prohibited (`none` selected),
  no provider (names `--ask`/`PTAH_ASK`/`[ask]`), provider failure, and
  end of input (stdin EOF on a non-terminal — a piped writer closed
  without answering).
- **Blocking is per-coroutine**: only the caller suspends; in-flight
  turns keep streaming and complete, sessions survive the ask (same
  subprocess serves later prompts), and a pending ask at script end
  keeps the run alive like an outstanding task. Concurrent asks
  serialize FIFO, attributed `ask {n} {script}`.
- **Provider selection is the operator's**, never the script's:
  `--ask` flag > `PTAH_ASK` env > project `[ask]` > user `[ask]` >
  auto-detect (stdin provider only when stdin *and* stdout are
  terminals — in CI, set `PTAH_ASK=stdin` explicitly; it works over
  pipes). Ctrl-C during an ask is run cancellation (exit 130/143), not
  an abort.
- **Ask prompts always render**, even under `--quiet` — a suppressed
  prompt is a hung run. The answer text is never re-echoed.

## Typed results, details

`resultSchema` injects one extra MCP server named `ptah` exposing a
single `result_submit` tool (agents that derive tool names call it
`mcp__ptah__result_submit`). The schema travels in the tool's
description/value — ptah never modifies your prompt text. Submissions
are validated against the schema; violations go back as a tool error the
agent can fix *inside the same turn*. Last accepted submission wins;
cancelled/timed-out turns discard it.

- Schemas compile eagerly at `session()`: an invalid schema or a remote
  `$ref` raises at your call site **before** any subprocess spawns.
- Any root shape works — `{ type = "string", enum = { "ship", "block" } }`
  submits plain strings. JSON `null` arrives as `nil`.
- Luau→JSON caveat: an empty table `{}` serializes as an *object*, not an
  array — write non-empty arrays (`enum = { "a" }`) or explicit array shapes.
- Agents that don't submit on their own may need the prompt to say so.

## Per-session config (models, etc.)

Agents expose per-session config (above all the *model*) via ACP config
options. Apply with `setConfig` calls immediately after `session()`
returns — before the first prompt — enumerate with `configOptions()`:

```lua
--!strict
local agent = ptah.agent("claude")
local reviewer = agent:session({ id = "rev" })
reviewer:setConfig("model", "claude-opus-4-5") -- before the first prompt
reviewer:setConfig("model", "claude-haiku-4-5") -- or between turns
for _, o in ipairs(reviewer:configOptions()) do
    ptah.log(("%s = %s"):format(o.id, tostring(o.currentValue)))
end
reviewer:close()
```

**Sequencing matters and is yours to control.** Some agents re-derive
dependent options when a driving option is set (opencode resets
`effort` whenever `model` is set), so set driving options first and
dependent options last — each awaited `setConfig` sees the state the
previous one returned. There is no constructor `config` table exactly
because a Luau table cannot express that order (`pairs()` order is
unspecified); passing one raises before any subprocess spawns.

**Option ids and value ids are agent-defined** — `"model"` belongs to the
agent you drive; never assume a hardcoded id exists. Enumerate
`configOptions()` first. Entries carry `id`, `name`, `type`
(`"select"` | `"boolean"`), `currentValue`, optional `category` (UX hint
only), and for selects an `options` array of `{ id, name, description? }`.
`setConfig` accepts strings (choice ids) or booleans, and is serialized
with turns — a call issued mid-turn waits, so changes apply strictly
between turns. On rejection it raises a catchable error naming the id
and carrying the agent's message; the session stays open (configure
right after creation, and close or let the run-end sweep reap on
error).

## Registry

TOML; project `.ptah/config.toml` (found upward from the invocation
directory) overrides `~/.config/ptah/config.toml` **per agent name**:

```toml
[agents.claude]
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
env = { ANTHROPIC_API_KEY = "${ANTHROPIC_API_KEY}" }
```

`${VAR}` interpolates from ptah's environment at resolve time (unset →
empty); `env` merges over the inherited environment. Process-level env
cannot vary per session — per-session model fan-out is exactly what
`setConfig` is for.

The registry's one global (non-agent) section is `[ask]`, selecting the
`ptah.ask` provider:

```toml
[ask]
provider = "stdin"   # or "none" to prohibit asking outright
```

`provider` is the only key; a missing key or unknown value fails
discovery naming the file, section, and accepted values. Across layers
the section replaces **wholesale** (project beats user in its entirety),
and it sits *under* `--ask` and `PTAH_ASK` in the selection chain.

Scripts read that same environment directly with `os.getenv(name)` (the
only env-read surface): it observes a snapshot taken when the run starts,
returns `nil` for unset variables (`os.getenv("X") or "fallback"` is the
idiom — an explicitly empty value returns `""`, distinct from unset), and
neither enumerates nor mutates anything.

## `ptah check` and pre-flight

`ptah check script.luau` verifies a script **without executing it** —
nothing runs, nothing spawns. Three passes, findings collected together:

1. **Compile** — same compiler `run` uses (compiled, never called).
2. **Static lints** — full-moon walk over the entry and every file
   reachable through *literal* `require("...")` strings: unknown literal
   `ptah.agent("name")` names against the discovered registry; literal
   require targets that don't resolve; missing `--!strict`; and unusable
   interaction — `ptah.ask(` call sites (any argument form) with the
   provider resolved to `none` or unresolvable off-terminal (the finding
   names `--ask`/`PTAH_ASK`/`[ask]`; no ask sites means no finding).
   Computed names/paths and aliased asks (`local f = ptah.ask`) are not
   linted.
3. **Typecheck** — `luau-lsp analyze` against the embedded definitions;
   must be on PATH or the check hard-fails (exit 2).

Exit `0` clean · `1` findings (warnings like `LocalUnused` don't fail) ·
`2` could not run. `ptah run` also pre-flights (compile + literal
require + literal agent-name + interaction) and fails with exit 1
**before the first agent spawns** — a literal require on a dead code
path still fails it (delete dead requires), and an ask on a dead branch
fails under `none`/unresolvable (allow a provider or remove the dead
ask).

## Pitfall checklist

- Missing `--!strict` on the entry (or a required module) → check finding.
- Implicit `PromptResult`→`string` coercion under `--!strict` → type
  error — write `r.text` (details under `PromptResult` above).
- Sharing one session across `parallel` workers → turns silently serialize
  (turn lock); pool sessions instead.
- `ptah.parallel` callback losing the item type → annotate the parameter
  (`function(item: string)`).
- Outcome narrowing works on locals — bind `local entry = outcomes[i]`
  (or a `for` variable) before `if entry.ok then entry.value …`.
- Typo'd *option* keys in a session-options literal are **not** flagged
  by luau-lsp — double-check `id`/`cwd`/`mcpServers`/`resultSchema`
  spelling by hand (invented result fields like `r.txt` *are* flagged).
- Setting a dependent option before its driving option (e.g. `effort`
  before `model` on opencode) — the agent's re-derivation silently
  reverts it; order your `setConfig` calls: driving options first.
- Empty Luau table `{}` in a schema means JSON object, not array.
- Bashisms in `ptah.exec` commands (`[[ ]]`, arrays, `$'…'`) — `/bin/sh`
  is POSIX-only; single-quote dynamic args (escape embedded quotes as
  `'\''`).
- Expecting a nonzero `ptah.exec` exit to fail the run — every exit
  code is data; branch on `exitCode` yourself.
- Unknown literal agent name → check finding and run pre-flight failure;
  registry miss at runtime raises at the `ptah.agent(...)` call.
- Expecting a value from `result` without a nil-check retry loop —
  degradation to `nil` is designed behavior, not an error.
