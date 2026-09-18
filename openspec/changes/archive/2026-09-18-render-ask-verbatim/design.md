## Context

See proposal.md — Why. Today `ask_prompt_line`/`ask_details_line` route ask
prose through `prompt_preview` (whitespace collapse +
`truncate_visible(LINE_BUDGET)`), the same path as ambient log lines. The
renderer already treats asks specially in one dimension (the `--quiet`
bypass); this change adds the second: fidelity. The renderer has no
terminal-width awareness and its line model is one timestamped `ptah_line`
per output row — both are kept as-is.

## Goals / Non-Goals

**Goals:**

- Ask prose (prompt + details) reaches the terminal byte-faithful in line
  structure: interior newlines and blank lines preserved, no collapse, no
  truncation.
- Keep the renderer's one-timestamped-line-per-row model — no wrapping, no
  width detection, no continuation lines without timestamps.
- Keep every other budget consumer (prompt lines, tool peeks, exec lines)
  untouched.

**Non-Goals:**

- Terminal-width-aware wrapping or reflowing (ask prose soft-wraps in the
  terminal like any long line).
- Any change to `LINE_BUDGET` itself, `truncate_visible`, or the collapse
  semantics used by non-ask lines.
- Any change to the record sink (already stores full text), the
  `AskRequested`/`AskResolved` events, the ask binding, or the stdin
  provider.
- Multiline ask *answers* (v1 contract: one line; unchanged).

## Decisions

**1. First prompt line rides the label line; continuations and details
indent uniformly (2 spaces).**
Alternatives: (a) whole prompt as a block *under* a bare label line — adds a
line to the common one-line case and separates the question from its
attribution; (b) prompt continuations at 2 spaces and details at 4 —
distinguishes the two fields visually but encodes a hierarchy the script
author didn't ask for and complicates the line builder. The chosen shape
keeps the common case identical to today (`ask 1 main.luau: Continue?`) and
degrades gracefully: everything after the first line reads as one prose
block under the attribution. Blank continuation lines render empty (no
2-space padding) so no output row is trailing-whitespace-only.

**2. No length cap at all — runaway ceiling rejected.**
The alternative was a large safety ceiling (a few KB) against a pathological
prompt. Rejected: asks come from script authors (not agents, not tool
output), the record already captures everything, the terminal soft-wraps
rather than breaking, and any ceiling is a fidelity cliff with an
unjustifiable constant — exactly the failure mode this change removes.
Verbatim means verbatim.

**3. Leading and trailing blank lines trimmed; interior structure
preserved.**
A prompt ending `"...?\n"` must not gain a dangling empty row before the
cue, and a prompt *beginning* with a blank line (`"\nQuestion?"`) or a
fully blank prompt (`""`) must not put an empty payload on the label
line (a trailing-whitespace row) — so leading blank lines are trimmed
the same way and a blank-after-trimming prompt renders the label line
with no text after its colon. Interior blank lines are structure the
author wrote — they stay. This is display-side only; the event payload
and the record keep the raw text.

**4. Continuation lines go through the same timestamped `ask_line` path.**
Each rendered row — including indented continuations — carries the
timestamp and `[ptah]` prefix. Alternative: timestamp only the first row of
a block (man-page style). Rejected: it introduces a second line archetype
into the renderer for a rare benefit; uniform rows keep the "timestamped
like every rendered line" contract trivially true and log scraping simple.

**5. The `> ` cue mechanics are unchanged.**
Prose lines are emitted with trailing newlines via the existing
`ask_line`; the cue is then written without a newline, exactly as today —
it lands alone on the row after the last prose line. Multi-line prose needs
no new cue handling.

## Risks / Trade-offs

- [A 1000+-character ask sprays many rows across the terminal] → Accepted:
  that is the author asking a long question; the human being asked needs to
  read it. `--quiet` cannot suppress it for the same reason it cannot
  today.
- [Continuation rows timestamped per line make a block visually chatty] →
  Accepted for model simplicity (decision 4); revisit only if real usage
  complains.
- [Unit tests pinning the old collapsed/truncated shape break] → Expected;
  they are replaced by verbatim-shape tests (see tasks.md).

## Migration Plan

Single-crate rendering change, no API/event/file-format movement: land the
renderer change with its tests in one commit. Rollback is revert-the-commit;
no persisted state interprets the old shape (the record never stored the
truncated form).
