## Why

`ptah.ask` is required interaction — the spec's own words when it exempts ask
lines from `--quiet` ("a suppressed prompt is a hung run"). Yet the terminal
renderer collapses the ask prompt's whitespace and truncates it at the shared
120 visible-char budget, the same treatment as ambient log noise. A human can
be asked to decide on a question they cannot fully read, and there is no
channel to see the rest: both `prompt` and `details` are clipped, so any ask
needing more than ~240 characters of context is unreadable in the terminal —
the one place the answer is typed. The `--quiet` bypass already established
that asks are interaction surfaces, not log lines; this change applies the
same principle to fidelity.

## What Changes

- Ask prompts render **verbatim**: authored newlines, blank lines, and
  whitespace runs are preserved; no whitespace collapse.
- Ask prose is **exempt from the shared 120-char truncation budget**: no `…`
  marker on ask lines, no length cap (see design: runaway ceiling considered
  and rejected).
- Rendering shape: the prompt's first line rides the label line
  (`ask {n} {script}: <first line>`); the prompt's continuation lines and the
  details render as indented verbatim lines beneath it.
- Each rendered line (continuations included) keeps the standard timestamp
  and `[ptah]` attribution, exactly like every other ask line.
- The `> ` input cue, the resolution line (action only, never the answer),
  the `--quiet` bypass, and `--no-color` behavior are unchanged.
- Prompt/tool/exec lines keep the shared budget and collapse semantics — the
  exemption is scoped to ask prose only.
- The run record is unchanged: it already stores full prompt/details text.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `render-logging`: the "Ask lines render under the ask label" requirement
  pins ask prompts to one collapsed, budget-truncated line; it is rewritten
  to a verbatim, budget-exempt contract. No other requirement in this spec
  changes.

## Impact

- `crates/ptah-render/src/lib.rs` — `ask_prompt_line`/`ask_details_line`
  reworked from `prompt_preview` (collapse + `truncate_visible`) to verbatim
  multi-line emission; unit tests that pinned the truncated shape are
  replaced.
- `crates/ptah-core/src/text.rs` — unchanged (`LINE_BUDGET` stays for
  prompt/tool/exec lines).
- No changes to `ptah-core` events, the ask binding, the stdin provider, the
  record sink, or any e2e surface (e2e asks use short prompts; no test pins
  the clipped shape outside the renderer unit tests).
- Not breaking: rendering-only change; no API, event, or file-format change.
