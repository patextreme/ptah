# Render Logging Capability Delta

## ADDED Requirements

### Requirement: Ask lines render under the ask label
The renderer SHALL render ask activity attributed to a reserved `ask` pseudo-label carrying the per-run ask number and the entry script's basename (reading as script activity, like exec lines, not as a named session). An issued ask SHALL render its prompt as one line (whitespace collapsed and truncated under the same visible-char budget as prompt lines) and, when details were provided, one further indented line for the details under the same mechanics, followed by an input cue. On resolution, one line SHALL carry the ask number and the action (`respond` or `abort`); the response text SHALL NOT be re-echoed by ptah (the terminal already shows what was typed). Ask lines SHALL render even under `--quiet`: a suppressed prompt is a hung run, so asks are required interaction, not machine noise — `--quiet` governs streaming and diagnostics only. Ask lines are timestamped like every rendered line and follow `--no-color`.

#### Scenario: Prompt and details render
- **WHEN** a script calls `ptah.ask({ prompt = "Blocked: how to continue?", details = "probe output …" })`
- **THEN** two ask-attributed lines render — the prompt line, then an indented details line — followed by an input cue

#### Scenario: Quiet keeps ask lines
- **WHEN** the same ask is issued in a `--quiet` run
- **THEN** the ask lines still render (while agent streaming stays suppressed)

#### Scenario: Resolution line carries the action, not the text
- **WHEN** an ask is answered `retry`
- **THEN** a resolution line naming the ask and `respond` renders, and the answer text does not appear in any ptah-rendered line

#### Scenario: Ask attribution
- **WHEN** the run's first ask is issued by the script `main.luau`
- **THEN** its lines are attributed to `ask 1` carrying `main.luau`
