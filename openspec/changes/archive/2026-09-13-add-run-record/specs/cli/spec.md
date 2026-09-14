## MODIFIED Requirements

### Requirement: Run subcommand executes a script
The `ptah` CLI SHALL provide `ptah run <script.luau>`, where `<script.luau>` is a positional required path to the entry Luau script. Each `ptah run` SHALL create a run record for the run, per the `run-record` capability.

#### Scenario: Successful run
- **WHEN** `ptah run script.luau` is invoked and the script completes without uncaught errors
- **THEN** the process exits with code 0

#### Scenario: Missing script argument
- **WHEN** `ptah run` is invoked without a positional path
- **THEN** the CLI prints a usage error and exits non-zero without executing anything

#### Scenario: Nonexistent script file
- **WHEN** the positional path does not exist on disk
- **THEN** the CLI prints an error naming the path and exits non-zero

#### Scenario: A run leaves a record
- **WHEN** `ptah run script.luau` runs in a project
- **THEN** a run record directory appears under `<project>/.ptah/runs/`

### Requirement: Output control flags
The CLI SHALL accept output flags: `--quiet` suppresses all streaming render and diagnostics **on the terminal**, `--verbose` shows runtime lifecycle diagnostics, a second verbosity level (`-vv`) additionally passes agent subprocess stderr through, and `--no-color` disables ANSI colors while keeping text prefixes. `--quiet` SHALL NOT suppress the run record: the record always receives the run's rendered stream at the run's verbosity, per the `run-record` capability, so the flag governs what the operator sees and never whether the run is recorded.

#### Scenario: Quiet flag
- **WHEN** a script runs with `--quiet`
- **THEN** no streaming output from agents is printed (output produced by the script itself via `print` still passes through)

#### Scenario: No-color degradation
- **WHEN** a script runs with `--no-color`
- **THEN** session output is still attributed by its text prefix but contains no ANSI escape sequences

#### Scenario: Quiet still records the run
- **WHEN** a script that prompts an agent runs with `--quiet`
- **THEN** the terminal stays silent for that streaming output and the run's `log` file contains it
