# Proposal: add-ptah-ask

## Why

Workflows hit blockers that only a human can resolve (e.g. an OpenSpec probe
that needs guidance mid-run). Today the only outlet is raising an error, which
unwinds the workflow and tears down the run: repository changes survive, but
the live continuation and the ACP session are lost. A human-resolvable blocker
is not a workflow failure — the workflow should pause, collect guidance, and
resume with the same agent session. (Issue #16.)

## What Changes

- New Luau primitive `ptah.ask({ prompt, details })`: suspends **only the
  calling coroutine** while a human answers. The ptah process, workflow state,
  and ACP sessions stay alive and streaming.
- Result is response-or-abort (`{action="respond", text} | {action="abort"}`);
  system failures (prohibited, no provider, provider failure, stdin EOF)
  **raise** with distinct messages. No timeout option in v1 — an ask blocks
  until answered, aborted, or the run is cancelled.
- New funded core port #6, `AskProvider` (`AskRequest`/`AskOutcome`), injected
  through `RunConfig` like `ProcessRunner` (the `shell-exec` precedent).
  `InteractionPolicy` and the headless permission posture are untouched.
- First provider: `stdin` — prompts render on stdout under pseudo-label
  `ask`, answers read from stdin lines; works over pipes when explicitly
  selected. Abort = `/abort` line or Ctrl-D on empty input; Ctrl-C keeps its
  existing meaning everywhere (run cancel, exit 130/143).
- Provider selection is an operator decision with precedence
  `--ask` > `PTAH_ASK` > project `[ask]` > user `[ask]` > auto-detect
  (TTY → `stdin`). `none` prohibits asking outright; it does not affect
  permission auto-allow. `[ask]` is the registry's first global section
  (whole-section replacement across layers); bad provider values fail fast.
- Static detection: a full-moon lint collects `ptah.ask` call sites over the
  literal require graph; `ptah check` and `run` pre-flight fail (exit 1) when
  ask is used and the provider resolves to `none` (prohibited) or to
  auto-detect with no TTY (unresolvable). The runtime check remains the source
  of truth.
- Ask lifecycle events (`AskRequested`/`AskResolved`) through `EventSink`;
  ask lines bypass `--quiet` (a suppressed prompt is a hung run).
- Durable suspension, later providers (webhook, Slack, TUI), and answer
  choices/multiline/timeout options are explicit non-goals / follow-ups.

## Capabilities

### New Capabilities

- `ask`: The `ptah.ask` human-interaction subsystem — the `AskProvider` port,
  the Luau binding contract (result/error semantics, coroutine suspension,
  serialization), the `stdin` provider, provider selection and the `none`
  posture.

### Modified Capabilities

- `scripting`: `ptah.ask` joins the `ptah` namespace; blocking-is-per-coroutine
  contract extends to asks; new runtime errors (prohibited / no provider /
  provider failure / EOF).
- `agent-registry`: first global section `[ask]` (`provider` key), validation
  of provider values, whole-section layer replacement (project over user).
- `cli`: `--ask=<stdin|none>` flag on `run` **and** `check`; `PTAH_ASK` env
  override; precedence chain and fail-fast on bad values.
- `script-checking`: ask call-site lint over the literal require graph;
  prohibited and unresolvable findings in `check` and `run` pre-flight.
- `render-logging`: ask prompt lines under pseudo-label `ask`, quiet-mode
  bypass, `AskRequested`/`AskResolved` event gating.
- `type-definitions`: `AskResult` type and `ask` field in `.ptah/ptah.d.luau`.

## Impact

- `crates/ptah-core` — new port + `AskRequest`/`AskOutcome` types, two new
  `SessionEvent` variants (core stays I/O-free; deps_guard unaffected).
- `crates/ptah-luau` — `ptah.ask` binding, `RunConfig` gains the injected
  provider, ask counter/attribution state.
- `crates/ptah-config` — parse/validate/merge the `[ask]` global section.
- `crates/ptah-check` — ask lint + resolution findings in check and preflight.
- `crates/ptah-render` — ask lines, quiet bypass, event rendering.
- `crates/ptah-cli` — composition root: stdin provider impl (owns stdin; libc
  isatty already available), flag/env plumbing, `ptah init` skeleton comment.
- `.ptah/ptah.d.luau`, `README.md`, `skills/ptah/SKILL.md`, `examples/ask.luau`
  + examples.rs harness variant (piped stdin), mock-agent unchanged (asks are
  host-side; tests use an injected fake provider and piped stdin).
- No new external dependencies; no breaking changes to existing scripts
  (`ptah.ask` is additive; scripts without it behave identically).
