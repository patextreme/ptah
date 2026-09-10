# ptah

> **Name origin:** *Ptah* is the Egyptian god of craftsmen and architects —
> the master builder who shaped the world by thinking it, then speaking it
> into being.

Luau-scripted multi-agent orchestration over the
[Agent Client Protocol](https://agentclientprotocol.com/) (ACP).

`ptah run script.luau` executes a small, versionable Luau script that drives
any ACP-speaking agent (Claude Code, Gemini CLI, Codex, …) over stdio.
Scripts look synchronous — `reply = session:prompt("…")` blocks the script,
not the runtime — so fan-outs, pipelines and watchdogs read like plain code.

```lua
--!strict
local claude = ptah.agent("claude")
local s = claude:session({ id = "reviewer" })
local r = s:prompt("Review src/main.rs for obvious bugs; be terse.")
ptah.log(tostring(r))       -- r.text; r.stopReason; r.usage.input …
s:close()
```

Fan out over many targets with a concurrency cap:

```lua
local outcomes = ptah.parallel(targets, function(t)
    local s = claude:session()
    local r = s:prompt("Summarize " .. t)
    s:close()
    return r.text
end, { concurrency = 2 })
```

Turns on a single session serialize — a `prompt` issued while a turn is in
flight waits for it — so overlapping fan-outs give each lane its own session;
the concurrency cap then bounds concurrent agent subprocesses.

## Install / build

```sh
nix build              # produces bin/ptah (crane, pinned nightly toolchain)
nix develop            # dev shell with the pinned toolchain
cargo build            # plain cargo works too
cargo test             # full suite; integration tests use the mock agent only
```

## CLI

```
ptah run <script.luau> [--quiet] [--verbose] [-vv] [--no-color] [--ask=<provider>]
ptah check <script.luau> [--no-color] [--ask=<provider>]
ptah types
ptah completions <shell>
ptah init
ptah --version
```

- `--quiet` — suppress streaming render and diagnostics (script `print` still passes; ask prompts still render — they are required interaction, not noise)
- `--verbose` — runtime lifecycle diagnostics
- `-vv` — additionally pass agent subprocess stderr through
- `--no-color` — drop ANSI colors, keep `[agent/session]` text prefixes
- `--ask=<stdin|none>` — ask provider for `ptah.ask` (on `run` **and**
  `check`): `stdin` reads answers from stdin, `none` prohibits asking.
  The same value set is read from the `PTAH_ASK` environment variable;
  precedence is `--ask` > `PTAH_ASK` > project `[ask]` > user `[ask]` >
  auto-detect (the `stdin` provider only when both stdin and stdout are
  terminals). Unknown values are usage errors (exit 2); an invalid
  `PTAH_ASK` fails the invocation with exit 2
- `ptah types` — print the Luau type definitions for the script API
  (see [Editor setup](#editor-setup)); needs no registry, script, or agents
- `ptah completions <shell>` — print a shell completion script for
  `bash`, `zsh`, `fish`, `elvish`, or `powershell` (see
  [Shell completions](#shell-completions)); unknown shells are usage
  errors
- `ptah init` — scaffold `./.ptah/` with the type definitions and a
  commented registry skeleton (see [Editor setup](#editor-setup));
  the config skeleton is created once and left alone, the definitions
  are synced to the installed binary

Exit codes: `0` on success, `1` on an uncaught script error or a never-observed
task error (printed to stderr), `2` on CLI/usage errors, `n` when the script
calls `ptah.exit(n)`, and `130`/`143` when the run is cancelled by SIGINT/
SIGTERM (teardown — including killing in-flight exec children — runs before
the exit). For `ptah check`, `1` means findings and `2` also covers
"check could not run" (see [Checking scripts](#checking-scripts)).

## Shell completions

`ptah completions <shell>` prints a completion script for `bash`, `zsh`,
`fish`, `elvish`, or `powershell` to stdout — nothing else — so it's a
plain redirect into your shell's completion directory. Scripts are
generated from the installed binary's own command tree, so they always
match your `ptah` version (re-run the install line after upgrading);
hidden internals never appear. An unknown or missing `<shell>` is a
usage error (exit 2). The nix flake package installs the `zsh`, `bash`,
and `fish` scripts into its output's standard completion directories
(`share/zsh/site-functions`, `share/bash-completion/completions`,
`share/fish/vendor_completions.d`), generated at build time from the
binary being packaged — so profile-based installs (NixOS, home-manager,
`nix profile`) get working completions automatically and the redirect
lines below are only needed for other installation paths (cargo, etc.).

```sh
# bash (bash-completion picks up files in this directory)
ptah completions bash > ~/.local/share/bash-completion/completions/ptah

# zsh (~/.zfunc must be on fpath before compinit)
ptah completions zsh > ~/.zfunc/_ptah

# fish
ptah completions fish > ~/.config/fish/completions/ptah.fish

# powershell (appends the registration to your profile)
ptah completions powershell >> $PROFILE

# elvish (appends the registration to your rc)
ptah completions elvish >> ~/.config/elvish/rc.elv
```

Completion is static: subcommands and flags complete; agent names live
inside scripts, not argv, so nothing dynamic is lost.

## Output format

Streaming output is plain stdout, one line per event, each prefixed with a
local wall-clock timestamp and the session attribution:

```
2026-08-25 21:07:33 [claude/reviewer] prompt: review the auth module for drift against the spec
2026-08-25 21:07:33 [claude/reviewer] tool: bash git status
2026-08-25 21:07:36 [claude/reviewer] tool: bash git status (completed, 2.9s)
2026-08-25 21:07:37 [claude/reviewer] tool: read src/render/mod.rs:118
2026-08-25 21:07:41 [claude/reviewer] Looks fine — two nits below.
2026-08-25 21:07:41 [ptah] log line from ptah.log
```

- Timestamps are always on (no flag): local `yyyy-mm-dd HH:MM:SS`, dimmed
  under color, plain text with `--no-color`. `--quiet` suppresses rendered
  output as before. Script `print` bypasses the renderer and is emitted
  verbatim.
- Every prompt renders one `prompt:` line at send time: the prompt text
  with whitespace runs collapsed to single spaces, truncated to a
  120-visible-char budget with a trailing `…` when cut. Suppressed by
  `--quiet` like all rendered output.
- A tool call renders at most two lines: the tool's title with an input
  peek appended when it enters `in_progress`, and the same title + peek
  with status and wall-clock duration when it settles —
  `tool: bash git status (completed, 2.9s)` (`1m 05.0s` past the minute).
  The peek is chosen from the tool call's own data, kind-aware: `execute`
  calls show the `command`/`cmd` string from the raw input; `read`/`edit`/
  `move`/`search`/`fetch`/`delete` calls show the first location as
  `path[:line]`, shortened relative to the session's cwd (`~/…` under the
  user's home, absolute otherwise); anything else shows the raw input as
  compact JSON. Peeks share the prompt line's 120-char budget and are
  skipped when the title already contains them (pi-acp-style bash titles
  are the command itself). Update lines resolve the title announced by the
  `tool_call`; the raw call id appears only when an update precedes its
  announcement. `pending` announcements and repeated identical statuses
  render nothing, so agents that resend the same status cannot flood the
  log.

## Agent registry

Agents are configured in TOML. Project entries (`.ptah/config.toml`, found
upward from the invocation directory) override user entries
(`~/.config/ptah/config.toml`) per agent name:

```toml
# ~/.config/ptah/config.toml
[agents.claude]
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
env = { ANTHROPIC_API_KEY = "${ANTHROPIC_API_KEY}" }
```

`${VAR}` interpolates from ptah's environment at resolve time (unset →
empty); `env` values are merged over the inherited environment. Scripts can
also pass an inline spec and skip the registry entirely:

```lua
local codex = ptah.agent({
    command = "npx",
    args = { "-y", "@agentclientprotocol/codex-acp@latest" },
})
```

### The `[ask]` section (global)

Registry files may also define `[ask]` — the registry's first global
(non-agent) section — selecting the ask provider for `ptah.ask` (see
[Asking a human](#asking-a-human-ptahask)):

```toml
# .ptah/config.toml (project layer)
[ask]
provider = "stdin"   # or "none" to prohibit asking outright
```

`provider` is the only key (validated at discovery: a missing key or an
unknown value fails with an error naming the file, the section, and the
accepted values). Across layers the section is replaced **wholesale** —
the project `[ask]` beats the user `[ask]` in its entirety, exactly like
per-agent-name replacement — and it sits under `--ask` and `PTAH_ASK` in
the selection precedence.

Any Anthropic-compatible provider works through the standard env. For
example, running Claude Code against Z.AI's GLM models:

```toml
[agents.glm]
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]

[agents.glm.env]
ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic"
ANTHROPIC_API_KEY = "${ZAI_API_KEY}"
ANTHROPIC_MODEL = "glm-4.6"
ANTHROPIC_SMALL_FAST_MODEL = "glm-4.5-air"
```

(Verified end-to-end: a real turn streams chunks + usage and completes with
`stopReason = "end_turn"`.)

### The `pi` agent (patched adapter in this flake)

The repo's project registry (`.ptah/config.toml`) ships a `pi` entry that
needs no local user config:

```toml
[agents.pi]
command = "pi-acp"
args = [ ]
env = { PI_NO_BELL = "1" }
```

`pi-acp` resolves via `PATH` from this repo's dev shell (`nix develop`),
which carries a patched build as a flake output:

- `packages.<system>.pi-acp` — `buildNpmPackage` of pi-acp 0.0.33 pinned at
  git rev `d1cffc0`, patched in-repo (`nix/packages/pi-acp/`): upstream
  pi-acp accepts ACP `session/new { mcpServers }` but never wires
  them into pi, so every `resultSchema` script would silently degrade to
  `result = nil`; the patch materializes stdio servers into a per-session
  `--mcp-config` file (mode 0600, removed at session end) so the bridge
  tool appears to the model as `ptah_result_submit`. Patch rationale and
  the rev-bump/rebase workflow: `nix/packages/pi-acp/README.md`.
- Outside the dev shell, run the adapter against a specific pi with
  `PI_ACP_PI_COMMAND` (an env entry in the agent's TOML):

  ```toml
  [agents.pi]
  command = "pi-acp"
  env = { PI_ACP_PI_COMMAND = "/nix/store/…-pi-0.84.3/bin/pi" }
  ```

**Prerequisite on the pi side:** the MCP adapter extension — `pi install
npm:pi-mcp-adapter` (verified with 2.29.0; check with `pi --help | grep
'--mcp-config'`). pi-acp probes this once per process and degrades
gracefully: without support (or for ACP http/sse servers) it warns —
"MCP: dropped N MCP server(s) …" — and drops them, and every turn
completes with `result = nil`, exactly ptah's documented degradation path.
Upstreaming the patch is out of scope; bumping the pinned rev requires
rebasing it by hand — see `nix/packages/pi-acp/README.md`.

## Permissions (headless posture)

`ptah` runs headless — nobody is there to be asked — so it answers every
`session/request_permission` by selecting an allow option the agent offered:
the first `AllowAlways` when one is offered, otherwise the first other
allow option (e.g. `AllowOnce`). A denied tool silently degrades output, so
allowing is the sane default for scripted runs; note that choosing
`AllowAlways` may persist an allow rule in the agent's own configuration
beyond the run (usually desirable for CI). When an offer contains no allow
option at all, ptah responds with an unsupported-method error. Everything
else agents may ask of a client — file access, terminal control,
elicitation — is answered with a JSON-RPC method-not-found error, so turns
never hang. Asking a *human* is a separate, deliberate channel —
[`ptah.ask`](#asking-a-human-ptahask) — and its `none` posture governs
asking only; the permission posture above is unaffected by it.

## The `ptah` namespace

| API | Description |
| --- | --- |
| `ptah.agent(name_or_spec)` | Agent factory (registry name or inline `{command=, args=, env=}` spec) |
| `agent:session({id=, cwd=, mcpServers=, resultSchema=})` | New session (own subprocess); `id` defaults to `s1, s2, …`; `resultSchema` declares a typed-result contract (see below); session config options are applied with `setConfig` after creation (see below) |
| `session:prompt(text, {timeoutMs=})` | One turn → `{ text, stopReason, usage, result }` (`result` is the turn's typed-result value, `nil` without one; `__tostring` → text; `text` is the turn's last agent message — see below); concurrent `prompt` calls on one session queue behind the in-flight turn |
| `session:cancel()` | Cancels the in-flight turn (returns `stopReason = "cancelled"`) |
| `session:close()` | Ends the session and reaps the agent process |
| `session:configOptions()` | Live per-session config options (empty table when the agent offers none) |
| `session:setConfig(id, value)` | Set a config option between turns — string (select choice id) or boolean value; raises on agent rejection |
| `ptah.spawn(fn)` → `task:await()` | Concurrent task; errors re-raise at the await site |
| `ptah.join({task, …})` | Wait for tasks → per-task `{ok, value}` / `{ok=false, error}` entries |
| `ptah.parallel(items, fn, {concurrency=})` | Parallel fan-out (default unlimited) → per-item outcome entries in item order |
| `ptah.exec(cmd, {timeoutMs=})` | Run a shell command via `/bin/sh -c` → `{ exitCode, stdout, stderr }` (any exit code is data; only could-not-run and timeout raise — see below) |
| `ptah.ask({prompt=, details=})` | Pause for a human answer → `{ action = "respond", text }` or `{ action = "abort" }` (suspends only the calling coroutine — see below) |
| `ptah.json.parse(s)` / `ptah.json.stringify(v, {indent=})` | Pure JSON decode (`null` → `nil`, raises on malformed input) / encode (string keys only) |
| `ptah.sleep(ms)` / `ptah.log(msg)` / `ptah.exit(code)` / `ptah.version` | Runtime helpers |

### Prompt text

`r.text` is the turn's **last agent message** — the final contiguous run
of streamed message text, where tool-call activity (`tool_call` /
`tool_call_update`) ends a message run. An agent that narrates ("Let me
check that file…"), runs tools, then answers ("The bug is on line 3")
yields only the answer in `r.text`, so `tostring(r)` reads as the reply;
the narration still streams to the terminal as it arrives. When a turn
ends on tool activity with no message after it, `r.text` falls back to
the turn's previous non-empty message; a turn with no agent message at
all yields `""`. A cancelled turn's `r.text` is `""` (its partial text
is as unreliable as its discarded typed result), and text from an
aborted turn never leaks into a later turn's `r.text` on the same
session.

### Typed results

`agent:session({ resultSchema = <schema> })` declares a typed result
contract: a JSON Schema as a plain Luau table. ptah then

- injects one extra MCP server into the agent's session, named `ptah`,
  exposing a single tool `result_submit` (agents that derive tool names
  call it `mcp__ptah__result_submit`). The declared schema travels in the
  tool's `value` argument — it never enters prompt text;
- sends every prompt verbatim (ptah never appends to or otherwise
  modifies the script's prompt text); the submit guidance — when to call
  and how the result is passed — lives in the `result_submit` tool
  description, which the agent discovers through normal tool listing.
  Agents that don't submit on their own may need the script to mention
  submission in the prompt itself;
- validates each submission against the schema and reports violations back
  as a tool error the agent can see and fix *inside the same turn* — the
  retry loop that makes typed results reliable.

`session:prompt()` outcomes gain a `result` field: the turn's last accepted
submission converted to a Luau value (`tables`, `strings`, `numbers`,
`booleans`; JSON `null` arrives as `nil`), or `nil` when the turn had no
accepted submission. Last submission wins; a fresh turn starts with an
empty slot; cancelled and timed-out turns discard what they had gathered.

```lua
--!strict
local agent = ptah.agent("claude")
local s = agent:session({
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

Schemas compile eagerly at `session()`: an invalid schema (or a remote
`$ref`, which is rejected so runs stay offline) raises a Lua error at your
call site before any subprocess spawns. Any root schema shape works — a
non-object schema such as `{ type = "string", enum = { "ship", "block" } }`
submits plain strings.

**Luau ↔ JSON notes.** The schema table converts from Luau to JSON with the
usual Lua caveats: an empty table `{}` serializes as an *object*, not an
array (write `enum = { "a" }` style non-empty arrays, or use explicit
array shapes), and JSON `integer`/`number` both arrive as Luau numbers.
JSON `null` in a submitted value arrives as `nil` (an explicitly submitted
`null` is indistinguishable from no submission).

**Degradation is designed, not exceptional.** Agents are free to ignore
suggested MCP servers, and sandboxes may block spawning the ptah binary.
In every such case prompts complete normally with `result = nil`, plus one
lifecycle log line (`--verbose`) noting the session ran without typed
results. Never an error, never a hang. Scripts that must have a value
write a nil-check retry loop, as above.

Scripts run in a sandboxed Luau environment: `string`, `table`, `math`,
`utf8`, `bit32`, `buffer`, `os.time`, `os.clock`, `os.getenv`, and `print` —
no file I/O, network, or debug facilities. `require` resolves `.luau` modules relative to
the requiring file with no directory boundary (`require("../shared/helper")`
reaches sibling trees); non-relative require strings (absolute paths, bare
module names, aliases) are rejected. Scripts are trusted code — they drive
agents with your full authority, and the sandbox limits the blast radius of
bugs, not malice. `os.getenv` fits that posture: it is the single
environment-read surface, observing a snapshot of ptah's environment taken
once when the run starts — read-only, not enumerable, and not mutable from
the script (`os.setenv` does not exist; variables whose values are not
valid UTF-8 read as unset). Environment values — secrets included — are
therefore script-readable, exactly like the shell profile that set them.
(One deviation: a restricted `coroutine` table containing
only `yield` remains visible because the embedded async runtime needs it;
the scheduling primitives are absent.)

The ambient globals expose no subprocess execution: world access arrives
through capabilities injected at the composition root. `ptah.exec` is
that door for the shell — implemented by a tokio runner the CLI always
injects (there is no gating flag or config switch, because running a
ptah script already implies arbitrary shell through the headless
allow-all agent posture; the injection seam exists so embedders of the
scripting runtime get a clean "no runner injected" error instead of an
ambient shell).

### Running shell commands: `ptah.exec`

Between agent turns there is deterministic work — list PRs, run a build,
format files. `ptah.exec(cmd, opts)` runs it inside the script and hands
the result back as data, so probabilistic agent steps and deterministic
pipeline steps compose:

```lua
--!strict
local r = ptah.exec("gh pr list --json number,title --limit 20")
local prs = ptah.json.parse(r.stdout)
for _, pr in ipairs(prs) do
    ptah.log(("#%d %s"):format(pr.number, pr.title))
end
```

The contract:

- **Invocation** is a single string run through `/bin/sh -c` — pipelines,
 redirections, and `&&` work. POSIX `sh` semantics only: `bashisms` are
 not guaranteed (`/bin/sh` is dash on Debian-family systems, bash
 elsewhere). With dynamic data, quote carefully — there is no portable
 `printf %q` under POSIX sh — single-quote arguments and escape embedded
 single quotes as `'\''` (an argv-array form may arrive later as an
 additive option).
- **Any exit code is data**: the call returns `{ exitCode, stdout,
 stderr }` and never raises for a failed command (`exitCode` maps a
 signal death to the shell's `128 + signal` convention). Only two
 conditions raise a catchable Lua error: the command could not run at
 all, and `timeoutMs` elapsing — the timeout error names the command
 and the budget, and the process *group* is killed first (mirroring the
 prompt-timeout contract).
- **Blocking is per-coroutine**: the calling script blocks, but spawned
 tasks and other sessions keep progressing (exec joins no `parallel`/
 `spawn` composition). No `timeoutMs` means no budget — the call waits
 for the command to exit, bounded only by outer cancellation.
- **The child inherits ptah's environment and working directory**;
 there are no `cwd`/`env` override options in v1. **Stdin is closed**
 (`/dev/null`): exec is non-interactive — a child that prompts fails
 fast on EOF instead of hanging or touching your terminal.
- **Captured output is yours**: nothing streams to the terminal; each
 exec renders one start line (the command) and one end line (exit code
 + duration, or a timeout/failed-to-run marker) as `[ptah] exec: …`
 script-activity lines, suppressed by `--quiet` like all rendered
 output.
- **Teardown is guaranteed**: a script error, `ptah.exit`, or run
 cancellation (Ctrl-C: the first SIGINT/SIGTERM runs the same teardown
 and exits `128+signal`; a second signal kills every still-running
 agent and exec child's process group before the immediate exit, which
 carries that signal's code — Unix only, signal handling is a
 Unix-only contract) kills every in-flight command's process group —
 no orphaned children outlive the run.

The session id `exec` is reserved (it attributes exec lifecycle lines at
the event sink): `agent:session({ id = "exec" })` is rejected at
session-options validation with a clear error — choose another id.

`ptah.json` exists for exactly this pattern: `parse(s)` decodes captured
 command output into Luau data (arrays as 1..n tables, objects as
 string-keyed tables, `null` as `nil`; malformed input raises),
 `stringify(v, { indent = n })` encodes compactly or with `n`-space
 indentation. It performs no I/O of its own.

### Asking a human: `ptah.ask`

Workflows hit blockers only a human can resolve — a probe that needs
guidance mid-run, a destructive step that wants a sanity check. Raising
an error unwinds the workflow and tears down the ACP session;
`ptah.ask(opts)` instead **pauses** the run and asks:

```lua
--!strict
local answer = ptah.ask({
    prompt = "Two candidates for the fix — which approach?",
    details = "a) retry with backoff  b) fail over to the replica",
})
if answer.action == "respond" then
    ptah.log("proceeding with: " .. answer.text)
else -- "abort": the human declined — handle it like any other value
    ptah.exit(1)
end
```

The contract:

- **`opts` is `{ prompt = <string>, details = <string>? }`** — `prompt`
  required (a usage error names it when missing), optional `details`
  rendered as one indented line under the prompt. No other options in
  v1; in particular **no timeout** — an ask blocks until answered,
  aborted, or the run is cancelled.
- **Response-or-abort is data**: `{ action = "respond", text = … }`
  (the answer line, unprocessed) or `{ action = "abort" }` (no `text`).
  With the `stdin` provider, a line of exactly `/abort` — or Ctrl-D at
  an empty prompt on a terminal — resolves abort. Abort is a normal
  result, not an error.
- **Four conditions raise** catchable, distinctly-worded errors:
  interaction prohibited (the `none` posture), no provider configured
  (nothing resolved the ask channel — the error names `--ask`,
  `PTAH_ASK`, and `[ask]`), provider failure, and end of input (the
  provider's input closed with no gesture — stdin EOF on a non-terminal,
  as when a piped writer closes without answering).
- **Blocking is per-coroutine**: only the calling coroutine suspends —
  other tasks, in-flight turns, and streaming keep progressing, and
  agent sessions survive the ask (an in-flight turn completes normally;
  the same session serves later prompts). A pending ask at script end
  keeps the run alive exactly like an outstanding task.
- **Concurrent asks serialize**: asks issued concurrently are delivered
  one at a time, first-come-first-served, each attributed `ask {n}
  {script}` (per-run number, entry script's basename) — prompts never
  overlap.
- **The provider is an operator decision**, never settable from script
code: `--ask` > `PTAH_ASK` > project `[ask]` > user `[ask]` >
  auto-detect (the `stdin` provider only when ptah's stdin **and**
  stdout are both terminals — over a pipe or in CI you must select
  explicitly, and the `stdin` provider works fine over pipes when you
  do). `none` prohibits asking outright; it does not touch the headless
  permission posture.
- **Ctrl-C keeps its meaning**: the first SIGINT/SIGTERM during a
  pending ask runs the normal run teardown and exits `130`/`143` — no
  abort is delivered (abort is a human answer, cancellation is
  process-level).
- **Prompts always render**: ask lines (prompt, details, resolution)
  are timestamped `[ptah]`-attributed script-activity lines that bypass
  `--quiet` — a suppressed prompt would be a hung run. The answer text
  is never re-echoed by ptah; the terminal already shows what was typed.

### Per-session config (models and more)

Agents increasingly expose per-session configuration — above all the model
— through ACP session config options. ptah advertises the
`session.configOptions` client capability (its only declared capability;
nothing interactive), captures the options each `session/new` response
advertises, and keeps them live as the agent pushes changes:

```lua
--!strict
local claude = ptah.agent("claude")
local opus = claude:session({ id = "reviewer" })
opus:setConfig("model", "claude-opus-4-5") -- before the first prompt
local haiku = claude:session({ id = "summarizer" })
haiku:setConfig("model", "claude-haiku-4-5")

for _, option in ipairs(opus:configOptions()) do
    ptah.log(("%s = %s"):format(option.id, tostring(option.currentValue)))
end

haiku:setConfig("model", "claude-opus-4-5") -- between turns
```

There is no constructor `config` table, and passing one is an error: a
Luau table cannot express application order (`pairs()` iteration order is
unspecified), and order is load-bearing for agents with dependent
options. A `config = { … }` key in the session-options table — populated
or empty — raises a catchable Lua error **before any agent subprocess
spawns**, with the migration spelled out in the message: apply config with
`session:setConfig(...)` after session creation, setting driving options
(like `model`) first.

`setConfig` is therefore also the creation-time path: configure
immediately after `session()` returns, before the first prompt. Order
your calls when the agent has dependent options — opencode, for
example, re-derives its `effort` option from the model on every `model`
set, so set `model` first and `effort` last; each awaited `setConfig`
sees the state the previous one returned, and the agent's response is
authoritative after each. Note the atomicity trade-off: unlike a
constructor-applied table, a rejected `setConfig` raises but leaves the
session open — the error is catchable, the run-end sweep reaps the
subprocess if the script aborts, and the error carries the config id and
the agent's message.

`configOptions()` returns the live option list: each entry has `id`,
`name`, `type` (`"select"` or `"boolean"`), `currentValue` (the selected
choice id, or the toggle state), an optional `category` (a UX hint — never
rely on it), and — for select options — an `options` array of
`{ id, name, description? }` choices.

`setConfig` accepts strings (select choice ids) or booleans, and is
serialized with prompt turns: a call issued while a turn is in flight
waits for it, so config changes apply strictly between turns. On agent
rejection — or when the agent does not support the method — `setConfig`
raises a catchable Lua error carrying the config id and the agent's
message; on success it returns `nil` and updates the live state.

**Option ids and value ids are agent-defined.** `"model"` and
`"claude-opus-4-5"` belong to the agent you are driving (e.g.
`@agentclientprotocol/claude-agent-acp` exposes `model`, `mode`, effort,
and subagent personas this way) — enumerate `configOptions()` first and
never assume a hardcoded id exists. The process-level alternative (env
vars like `ANTHROPIC_MODEL` in the registry) cannot vary per session; the
fan-out above — one agent, two models — is exactly what `setConfig`
before the first prompt is for. The [model-fanout
example](examples/model-fanout.luau) shows the full pattern; successful
sets and agent-pushed changes each render one lifecycle line
(`--verbose`).

## Checking scripts

`ptah check` verifies a script **without executing it** — no top-level
code runs, no required module loads, no agent subprocess spawns:

```sh
ptah check my_script.luau
```

Three passes run, findings are collected together (never fail-fast), and
each in-process finding prints to stderr as `path:line:col: message`
followed by a summary line (`--no-color` drops the ANSI coloring):

1. **Compile** — the entry compiles under the same Luau compiler `run`
   uses (compiled, never called). Syntax errors surface here with a
   line number; module-level syntax errors surface in the next pass.
2. **Static lints** — a full-moon AST walk over the entry and every file
   reachable through literal `require("...")` string arguments:
   unknown literal `ptah.agent("name")` names against the discovered
   registry; literal require targets that don't resolve under ptah's
   rules (`.luau`/`.lua`/`init.luau`, relative to the requiring file); a
   missing leading `--!strict` directive in the entry or any reachable
   file; and **unusable interaction** — `ptah.ask(` member calls (any
   argument form) combined with a resolved provider of `none`
   (prohibited) or no resolution at all off-terminal (the CI case): a
   finding naming the remedies (`--ask`, `PTAH_ASK`, `[ask]`). With no
   ask call sites, the posture produces no finding. Computed require
   paths, computed agent names, and aliased asks
   (`local f = ptah.ask`) are not linted — only literal shapes; the
   runtime check is the source of truth.
3. **Typecheck** — `luau-lsp analyze` (found on `PATH`) runs against the
   installed binary's embedded definitions; its diagnostics pass through
   verbatim. luau-lsp must be installed (`nix develop` ships it; see
   [luau-lsp](https://github.com/luau-lsp/luau-lsp)) — a missing binary
   is a hard error, not a silent skip.

Exit codes: `0` all passes clean · `1` findings (including luau-lsp
errors; warnings like `LocalUnused` don't fail) · `2` the check could not
run (missing/unreadable script, registry discovery failure, luau-lsp
absent).

`ptah run` also pre-flights every script in-process (compile + literal
require + literal agent-name + interaction lints — no strictness
enforcement, no luau-lsp) and fails the run with exit 1 before the first
agent spawns. The interaction lint resolves the ask provider under the
full selection chain, so `PTAH_ASK=stdin` (or `--ask=stdin`) keeps an
ask-using script runnable in CI. Scripts using computed require paths or
agent names run exactly as before. Known trade-offs: a literal require on
a code path that never executes at runtime still fails the pre-flight —
delete the dead require; and an ask on a dead branch fails under `none` —
allow a provider or remove the dead ask.

## Editor setup

`ptah init` is the front door: it scaffolds `./.ptah/` in the current
directory with exactly two files —

- `.ptah/ptah.d.luau` — the Luau type definitions for the script API,
  byte-identical to `ptah types` output (version header included), so
  they always match the installed binary;
- `.ptah/config.toml` — a fully commented agent-registry skeleton
  (see the comments there for the two-layer discovery and `${VAR}`
  interpolation rules).

The two files have different ownership. `.ptah/config.toml` is
user-authored: created once, reported as skipped on every later run,
never modified. `.ptah/ptah.d.luau` is a derived artifact of the
installed binary: init syncs it — created when absent, overwritten
whenever its bytes differ from the current binary's output, reported
`up to date` when it already matches. So re-running `ptah init` after
upgrading ptah is the primary way to refresh the definitions. The
documented alternative — for scripting, or refreshing without touching
config — is:

```sh
ptah types > .ptah/ptah.d.luau
```

Scripts get completion, hover, and type checking — plus sandbox
violations flagged before a run — by pointing
[luau-lsp](https://github.com/luau-lsp/luau-lsp) at `.ptah/ptah.d.luau`.
Start scripts with `--!strict` for full checking (the
[bundled examples](examples/) do). The definitions also model the sandbox —
`os` trimmed to `time`/`clock`/`getenv`, `coroutine` to `yield`, and
`loadstring`/`collectgarbage` unavailable — so editor-approved code cannot
reach a global the runtime poisons. Definitions apply workspace-wide, so
keep them out of mixed Luau projects you don't run under ptah.

Helix needs no per-user setup: the repo ships `.helix/languages.toml`,
which points luau-lsp at `.ptah/ptah.d.luau` (standard platform) for
any file in this workspace. Other editors — configure your own (VS Code
luau-lsp extension settings; "standard" platform, not
Roblox):

```jsonc
{
  "luau-lsp.platform.type": "standard",
  "luau-lsp.types.definitionFiles": [".ptah/ptah.d.luau"]
}
```

Neovim (nvim-lspconfig equivalent):

```lua
require("lspconfig").luau_lsp.setup({
  settings = {
    ["luau-lsp"] = {
      platform = { type = "standard" },
      types = { definitionFiles = { ".ptah/ptah.d.luau" } },
    },
  },
})
```

Known residuals of the definitions (none affect execution):

- generic `ptah.parallel` callbacks occasionally need an explicit parameter
  annotation (`function(item: string) …`) for the item type to propagate;
- the `tostring(r)` prompt-result sugar is not covered by the definitions —
  use `r.text` where the type checker wants a string;
- outcome narrowing (`if entry.ok then entry.value …`) works on locals —
  bind the outcome entry to a variable first;
- typo'd *option* keys in a session-options table literal are not flagged
  by current luau-lsp (a table-literal excess-key limitation) — invented
  *outcome* fields (`r.txt`) are flagged, double-check option names by
  hand; a `config` key is the removed constructor option and is rejected
  at runtime, pre-spawn.

Relative requires carry no residual: luau-lsp and ptah resolve them
identically — from the requiring file, with no directory boundary.

## Examples

See [`examples/`](examples/) — sequential review, fan-out with a concurrency
cap, per-session model fan-out, a watchdog cancel, typed results with a
retry loop, an exec pipeline (deterministic shell steps + JSON around one
agent turn), a mid-run human ask feeding an agent turn
(`PTAH_ASK=stdin ptah run examples/ask.luau`), and two sibling workflows
sharing a helper through a cross-tree require — and run them against the
bundled mock agent:

```sh
mkdir -p .ptah
cat > .ptah/config.toml <<'EOF'
[agents.demo]
command = "target/debug/mock-agent"
args = []
EOF
ptah run examples/sequential_review.luau
```

## Factory Components

[`factory-components/`](factory-components/) is the shared workflow
library: repo-agnostic stdlib helpers (`std/`) — the typed boolean
judge (`predicate`), the GitHub CLI transport (`gh`), and the repo-loop
skeleton (`daemon`) — plus
composable workflow components (`components/`) such as the openspec
lifecycle and a PR review loop. The scripts under
`.ptah/workflows/` are shims over it (dogfooding is what keeps the copies
from drifting).

The library is consumed as **source**: mount the tree wherever you
like (nix flake input + symlink, git submodule, vendored copy) and
write a shim — the only workflow code your repo owns:

```lua
--!strict
local openspec = require("./vendor/factory-components/components/openspec/component")

local ops = openspec.new({
	agent = ptah.agent("claude"),       -- work agent handle
	judgeAgent = ptah.agent("claude"),  -- judge agent handle (a small/fast model is ideal)
	sessionConfig = { { id = "model", value = "claude-opus-4-5" } },
	judgeSessionConfig = { { id = "model", value = "claude-haiku-4-5" } },
})

ops:groom("add-auth")
```

The contract that makes the mount work anywhere:

- **Mount-point freedom** — `require` is relative to the requiring
  file and may traverse outside the shim's directory, and library
  modules only require within their own tree, so the mount location is
  your free choice (a read-only nix store path included).
- **Data config plus agent handles** — every component field is data
  (strings, numbers, booleans) or a ptah runtime handle the component's
  config type declares (`agent: Agent`); functions are not
  configuration. Per-call data (a change name, a PR URL) is a method
  argument.
- **`ptah check` is the compatibility gate** — every module is
  `--!strict` and every component exports its `Config` type, so your
  shim's config is type-checked against the component's type when you
  run `ptah check` — a mistyped field is a finding naming the field.
  When you bump the mounted source, the check is what catches the
  break.

See [`factory-components/README.md`](factory-components/README.md) for
the full contract and each component's README for its declared
environment requirements; the distribution decision (source mount, no
registry, no lockfile) is recorded in the archived
[`factory-components`](openspec/changes/archive/2026-09-04-factory-components/)
change.

## Development

- `crates/ptah-cli/src/bin/mock-agent/` — a scriptable ACP agent (with an MCP client for
  suggested servers) used by the offline test suite (`MOCK_CHUNKS`,
  `MOCK_HANG`, `MOCK_PERMISSION` (`once`/`always`/`reject`), `MOCK_TOOL`,
  `MOCK_TOOL_FLOW` (status-sequence replay), `MOCK_PLAN`, `MOCK_USAGE`,
  `MOCK_STDERR`, `MOCK_DELAY_MS`, `MOCK_SUBMIT`,
  `MOCK_SUBMIT_BAD`, `MOCK_SUBMIT_ONCE`, `MOCK_SUBMIT_MATCH`
  (prompt-content-keyed submissions), `MOCK_NO_MCP`, `MOCK_ECHO_MCP`,
  `MOCK_MCP_LIST`, `MOCK_CONFIG_OPTIONS`, `MOCK_CONFIG_REJECT`,
  `MOCK_CONFIG_UPDATE`, `MOCK_CONFIG_ECHO`, …).
- `nix flake check` runs the entire suite in the sandbox.
- `nix/packages/pi-acp/` — the patched pi-acp adapter as a self-contained
  package directory: `default.nix` (pinned rev, v0.0.33), `mcp-config.patch`,
  and `README.md` with the patch rationale and rebase workflow (see "The
  `pi` agent" above). The patch is developed in a gitignored `.work/pi-acp`
  clone of the pinned rev and exported with `git diff`.
- Toolchain: pinned nightly in `rust-toolchain.toml`, consumed by the oxalica
  overlay for devshell and crane builds.
- **NixOS note:** vendor binaries shipped inside npm packages (e.g. the
  Claude Code executable) are dynamically linked against a generic loader.
Run them via a wrapper that invokes a nix glibc loader explicitly, and point
  the adapter at it with `CLAUDE_CODE_EXECUTABLE`:

  ```sh
  #!/bin/sh
  exec /nix/store/…-glibc/lib/ld-linux-x86-64.so.2 \
    ~/.npm/_npx/…/claude-agent-sdk-linux-x64/claude "$@"
  ```
