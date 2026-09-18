## MODIFIED Requirements

### Requirement: Ask lines render under the ask label
The renderer SHALL render ask activity attributed to a reserved `ask`
pseudo-label carrying the per-run ask number and the entry script's basename
(reading as script activity, like exec lines, not as a named session). An
issued ask SHALL render its prompt with the prompt's first line on the label
line (`{label}: {first line}`), each further line of the prompt as one
indented line beneath it, and — when details were provided — each line of the
details as one indented line beneath the prompt, followed by an input cue.
Ask prose SHALL render verbatim: authored line structure (interior newlines
and blank lines) preserved with no whitespace collapse, leading and
trailing blank lines trimmed, and no truncation — ask lines are exempt
from the shared visible-char budget that governs prompt lines, because
asks are required interaction, and an unreadable prompt is a hung run in
exactly the way a suppressed one is (the same principle as the `--quiet`
bypass). A prompt that is blank after trimming (an empty or all-blank
prompt) renders the label line with no text after its colon. On
resolution, one line SHALL carry the ask number and the action (`respond` or
`abort`); the response text SHALL NOT be re-echoed by ptah (the terminal
already shows what was typed). Ask lines SHALL render even under `--quiet`:
a suppressed prompt is a hung run, so asks are required interaction, not
machine noise — `--quiet` governs streaming and diagnostics only. Ask lines
are timestamped like every rendered line — each indented continuation line
included, each carrying the timestamp — and follow `--no-color`.

#### Scenario: Prompt and details render
- **WHEN** a script calls `ptah.ask({ prompt = "Blocked: how to continue?", details = "probe output …" })`
- **THEN** two ask-attributed lines render — the prompt line, then an indented details line — followed by an input cue

#### Scenario: Long ask prompt renders in full
- **WHEN** a script issues an ask whose prompt (or details) is longer than the shared visible-char budget that truncates prompt lines
- **THEN** the ask prose renders in full, with no `…` truncation marker on any ask line

#### Scenario: Multi-line ask prose keeps its line structure
- **WHEN** a script issues an ask whose prompt spans multiple lines (for example a question followed by a numbered list)
- **THEN** the prompt's first line renders on the label line and each further authored line renders as its own indented line, with interior blank lines rendered as empty lines (no indent padding)

#### Scenario: Quiet keeps ask lines
- **WHEN** the same ask is issued in a `--quiet` run
- **THEN** the ask lines still render (while agent streaming stays suppressed)

#### Scenario: Resolution line carries the action, not the text
- **WHEN** an ask is answered `retry`
- **THEN** a resolution line naming the ask and `respond` renders, and the answer text does not appear in any ptah-rendered line

#### Scenario: Ask attribution
- **WHEN** the run's first ask is issued by the script `main.luau`
- **THEN** its lines are attributed to `ask 1` carrying `main.luau`
