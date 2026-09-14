# Proposal: add-run-record

## Why

A headless run leaves exactly one durable artifact today: whatever the operator
redirected stdout into (`ptah run main.luau > out.log`). That log is anonymous —
no run identity, no script path, no exit status — and a shell redirect can lose
its tail on abnormal death, cannot separate ptah's rendered stream from the
script's raw `print`, and says nothing about which agent sessions ran. ptah
renders no transcripts and cannot resume a session, so for a headless run the
log *is* the audit trail, and it is least trustworthy exactly when it matters
most. Meanwhile `.ptah/workflows/README.md` mandates the entrypoint basename,
so ptah's own attribution (`[ptah] ask 1 main.luau: …`) identifies *a* workflow,
never *which* one. (Issue #24.)

## What Changes

- **Run identity and record.** Every `ptah run` mints a **run id**
  (`20260912143224-4821` — UTC compact timestamp plus a short random suffix,
  so a directory listing sorts chronologically) and creates a **run record** at
  `<project>/.ptah/runs/<id>/` holding `log` and `run.json`. The project is the
  nearest ancestor directory containing `.ptah/`, falling back to the invocation
  directory; there is no global or overridable state location.
- **Self-ignoring, atomically.** The record directory and a
  `.ptah/runs/.gitignore` containing `*` are created as one unit, and a record
  whose ignore file cannot be written is not created at all. This is required,
  not cosmetic: `.ptah/` is legitimately a committed directory (the
  `package-management` capability has users commit `.ptah/pesde.toml` and
  `.ptah/pesde.lock`), and workflows routinely `git add -A` before committing.
- **The log is a superset of the terminal.** `log` is always written, at the
  run's verbosity but never silenced: `--quiet` suppresses the terminal render
  only, and `-v` adds lifecycle diagnostics to the file too. Ask lines and
  `ptah.log` are included. The script's own `print` output is not — it never
  passes through the renderer, so capturing it would mean capturing process
  stdout rather than ptah's rendered stream.
- **`run.json` is rewritten, never assembled at exit.** Atomic `tmp` + rename at
  run start, each session becoming ready, each ask request and resolution, and
  every terminal transition (clean finish, uncaught script error, undelivered
  task error, `ptah.exit`, SIGINT/SIGTERM teardown). A run killed by SIGKILL
  (or hard-exited by a second termination signal during teardown) therefore
  still leaves a record naming its script, argv, cwd, start time, and
  the agents that had started. It carries `status` ∈ `{running, ok, failed,
  cancelled}` (cancellation taken from the runtime's report of how the run
  ended, never inferred from the exit code, so `ptah.exit(130)` is a failure),
  the process exit code, the terminal error message when there was
  one, per-session `{label, agent, command/args templates, env key names, ACP
  session id}`, and per-ask Q&A. A `schema_version` field carries the layout
  forward.
- **The record pins invocation shape, never secrets.** `env` values are never
  persisted (key names only), and `command`/`args` are recorded *before*
  `${VAR}` interpolation — a resolved arg list can literally contain an API key.
  The authored shape is read through a new `Registry::raw` accessor (the config
  model otherwise resolves `${VAR}` eagerly), and an inline `ptah.agent({...})`
  spec contributes its authored table values; the recorded agent name is the
  registry name, or the authored command for an inline spec.
- **One line at run start** names the record path, at default verbosity, so the
  operator does not have to `ls` for it. It is emitted only after the directory
  and `.gitignore` exist, so it can never name a record that does not.
- **Structured session readiness.** Session readiness is reported to sinks as
  a dedicated structured event (`SessionReady`) carrying the session's label,
  its agent name, its authored invocation shape (pre-interpolation
  `command`/`args` and env key names), and the agent-assigned ACP session id;
  the rendered `session ready` line is produced by the renderer from that
  event rather than by the ACP driver. The record sink therefore reads every
  fact it needs from the event instead of parsing rendered wording. This
  reverses a non-goal of `expose-acp-session-id` ("a `SessionReady` variant
  would serve only a hypothetical TUI adapter") because that premise is now
  false — a concrete sink needs it.
- **Failure is best-effort.** An unwritable record location (read-only
  checkout, `.ptah` owned by someone else) fails the record, not the run: no
  record, one warning on stderr that prints even under `--quiet`, run proceeds.
  A mid-run write failure disables the record, warns once, and never aborts.
- **Explicit non-goals / follow-ups:** retention and pruning; a
  `PTAH_STATE_DIR`/`--state-dir` override; the `ptah runs list|show|prune`
  verbs; the entry-script snapshot; entry/closure content hashes; git
  branch/commit/dirty metadata; per-session ACP transcripts (`sessions/`);
  `ptah run --resume <id>`; a script-visible `ptah.record(...)` API; and run
  records for `check`/`init`/`package`. The layout is chosen so the v2
  transcript and resume work can land on it, but nothing here depends on them.

## Capabilities

### New Capabilities

- `run-record`: the persisted record of one run — id minting, the record's
  location and layout, its self-ignoring `.gitignore`, what `log` and
  `run.json` contain, the rewrite triggers, the secrets rule, and the
  best-effort failure posture.

### Modified Capabilities

- `cli`: `run` creates a run record; the "Output control flags" requirement
  pins `--quiet` as terminal-only, with the record still receiving everything.
- `render-logging`: a new requirement for the run-start line naming the
  record; the session-ready requirement widens from the rendered line to the
  event payload that carries the id.

## Impact

- `crates/ptah-render` — a new `record` module owning the run id, directory
  creation and the self-ignoring `.gitignore`, the `log` writer, the
  `run.json` schema and its atomic rewrites; plus the `SessionReady` arm that
  renders the existing ready line (verbose-only, byte-identical). No `gix`, no
  new external dependencies.
- `crates/ptah-core` — the new `SessionEvent::SessionReady` variant
  (`events.rs`), matching the invariant that payloads carry structured facts
  (the module doc already states that rule), and a `Registry::raw` accessor
  (`config/mod.rs`) exposing the pre-interpolation spec, since `resolve`/
  `resolve_with` otherwise interpolate `${VAR}` eagerly.
- `crates/ptah-luau` — emits `SessionReady` once the session handle exists
  (label, agent name, authored shape, ACP id), reading the authored shape
  through `Registry::raw` for named agents and from the authored table for
  inline specs; and reports cancellation on `RunOutcome`, so the record can
  tell a terminating signal from a script's chosen exit code.
- `crates/ptah-acp` — drops its rendered `session ready` lifecycle emission
  (the renderer now reconstructs that line from `SessionReady`); no wire change.
- `crates/ptah-cli` — composition root: mint the id before the renderer is
  built, construct the fan-out sink feeding both renderers, emit the run-start
  line on both concrete renderers, observe session and ask events for rewrite
  triggers, and own the warning path when the record cannot be written.
- Docs: a README section on `.ptah/runs/`; `skills/ptah/SKILL.md` stops
  instructing scripts to hand-embed `session:sessionId()` into `ptah.ask`
  details for correlation and points at the record instead (the `sessionId()`
  method itself stays, for scripts that genuinely use the id).
- Not affected: `agent-sessions` (no wire behavior changes — ptah only carries
  a value it already receives, the same reasoning `expose-acp-session-id`
  used), `scripting` (additive, no script API change), `agent-registry` (no
  config change), `ask`, `shell-exec`, `typed-results`, `package-management`.
- Tests: ptah-render unit coverage for id minting, the ignore file, the
  superset rule under `--quiet`/`-v`, and each rewrite trigger; ptah-cli
  integration coverage for the start line, the best-effort failure warning,
  and a record surviving an abnormal exit.
- No new CLI surface, and no new crates in the dependency tree: the run id's
  randomness comes from `getrandom` 0.4, already resolved in the lock via
  `pesde`'s `gix`/`tempfile`/`uuid` stack, declared explicitly in the workspace
  table with its version pinned rather than relied on implicitly.
