# Design: add-run-record

## Context

See `proposal.md` — Why. The current state that shapes the approach:

- **One output path exists and it is a renderer.** `crates/ptah-render` owns a
  single `Mutex<BufWriter<Box<dyn Write + Send>>>` over stdout, flushed per
  line, shared by every session. The composition root builds it once
  (`crates/ptah-cli/src/cli.rs:658`) and hands it to the runtime as
  `Arc<dyn EventSink>`. `Renderer::with_writer` and `RenderOptions` are already
  public, so a second renderer needs no new API.
- **The sink port is small and sync.** `EventSink` (`ptah-core/src/ports.rs:245`)
  is `Send + Sync` with two unlabelled methods, `emit` and `script_log`;
  dispatch is a direct, blocking call. This change adds no port.
- **Events carry structure on purpose.** `ptah-core/src/events.rs` states the
  rule: "Payloads carry the structured facts (ids, kinds, statuses, counts) so
  a TUI can track state without parsing display strings". `ToolLine` is the
  precedent — "the fully formatted body plus the structured facts it was built
  from". `SessionEvent::Lifecycle { message }` is the one variant that breaks
  the rule, and it is where the ACP session id hides today.
- **ptah writes almost nothing at runtime.** No code in the workspace creates a
  `.gitignore`; `ptah init` scaffolds `.ptah/` but only once. The record is the
  first thing ptah writes during a run.
- **`.ptah/` is legitimately committed state.** `package-management` requires
  that `pesde.toml` and `pesde.lock` be committed while `luau_packages/` and
  `.pesde/` are ignored, and that spec already fixes "the project root (the
  directory containing `.ptah/`)". Workflows also `git add -A` as routine.
- **Two upward searches already exist and disagree.** `ptah-config::discover`
  stops at the nearest `.ptah/config.toml`; `ptah-pesde`'s project discovery
  accepts `config.toml` *or* `pesde.toml`.
- **Dependencies already in the tree but undeclared:** `getrandom` 0.4 (via
  `pesde`'s `gix`/`tempfile`/`uuid` stack), `sha2`. `jiff` is the declared clock.
- **`print` bypasses all of this.** Luau's `print` writes to the process's
  stdout through mlua, never through the renderer — the `cli` spec already
  states script `print` output "does not pass through the renderer".

## Goals / Non-Goals

**Goals:**

- Make a headless run self-describing: identity, what it ran, how it ended —
  durable without a shell redirect and trustworthy when the run dies abnormally.
- One ordered stream, one writer path: the record must not introduce a second
  ordering authority that can interleave with the terminal's.
- Keep the change inside the existing hexagonal arrows: no new port, no new
  crate, no dependency direction changes.

**Non-Goals** (each deliberately deferred; the layout is chosen so they *can*
land, but nothing here depends on them):

- **Retention / pruning.** Records accumulate; the operator deletes by hand.
  Deferred because retention needs a policy surface (a `[runs]` config section
  or flags) that is its own conversation.
- **`PTAH_STATE_DIR` / `--state-dir`.** No global or overridable state location.
  Deferred for simplicity; the accepted cost is that runs in read-only
  checkouts get no record (see Decisions — best-effort).
- **`ptah runs list|show|prune`.** The record plus the run-start line is the
  v1 interface. Named here so a future reader knows the directory is not
  considered a finished interface, only a finished *artifact*.
- **Entry-script snapshot and content hash.** With no snapshot, no hash, and no
  git metadata, **nothing in the record pins what code ran** — only where the
  script lived. This is a knowingly accepted gap: the record answers "when,
  which script, which agents, how did it end", not "which revision of the
  code". The first thing to add if that changes is the entry-script hash, and
  the second is git commit + dirty.
- **Per-session ACP transcripts (`sessions/`) and `ptah run --resume <id>`.**
  The real long-term payoff, explicitly v2.
- **A script-visible `ptah.record(...)` or run-id accessor.** The record is an
  output sink, not a capability (see Decisions).
- **Run records for `check`/`init`/`package`.**

## Decisions

- **The record is a runtime-owned output sink, and scripts get nothing.** No
  run id in the script environment, no `ptah.record`, no path accessor. The
  sandbox's I/O-freedom is a load-bearing property (`ptah-luau` blocks I/O, the
  debug library, and `loadstring`), and the moment the run directory becomes
  readable or writable from script code it is a filesystem surface that dodges
  the sandbox. Alternative rejected: exposing the id as `ptah.runId` so scripts
  could log it themselves — it buys nothing the `log` and `run.json` don't
  already carry, and it is a one-way door.

- **`--quiet` means "terminal only", and the record is the superset.** The file
  always receives the run's rendered stream, at the run's verbosity, never
  silenced. This is the same reasoning the `ask` capability already applied to
  ask lines ("a suppressed prompt is a hung run"), generalised: an operator
  silencing their terminal is not asking to be un-auditable. Alternative
  rejected: `--quiet` silencing the record too, which would make the artifact
  the least reliable thing in the system precisely when runs are automated.

- **Two renderers behind a fan-out sink, not a mirroring writer inside
  `Renderer`.** The composition root wraps two `EventSink`s: the terminal
  renderer built from the run's flags, and a record renderer built with
  `{ quiet: false, no_color: true, verbosity: terminal's }` writing into the
  run's `log`. The issue's verbosity model then falls out of `RenderOptions`
  rather than being re-derived. The alternative — giving `Renderer` an optional
  second writer — requires restructuring every `--quiet` early-return so a line
  is computed before deciding which writers receive it, i.e. rewriting the
  quiet semantics in the most heavily tested code in the tree. Accepted costs:
  two locks (each stream is internally consistent; there is no cross-stream
  atomicity, which nothing needs) and one timestamp computation per renderer,
  so a line straddling a second boundary can differ by a second between
  terminal and file.

- **The record adapter lives in `ptah-render`.** A record is an output adapter
  that sits beside the renderer and shares both `RenderOptions` and
  `Renderer::with_writer`. Alternatives rejected: a new `ptah-run-record` crate
  (a ninth crate, new flake wiring, for one cohesive module) and a module in
  the `ptah-cli` composition root (AGENTS.md keeps the root a *composer* of
  adapters — `FsConfigSource` lives in `ptah-config`, the ACP transport in
  `ptah-acp`). Consequence of the traceability cuts: `ptah-render` needs
  neither `gix` nor `sha2`, so it stays a pure formatting/writing crate.

- **The run id is `yyyymmddhhmmss-<decimal>`, minted in UTC.** `20260912143224-4821`.
  Alternatives: ULID (an opaque new crate for a format nobody here reads) and a
  `T`/`Z`-decorated ISO string (the first draft; rejected as needlessly noisy in
  a filename). UTC rather than local time is the important half: the renderer
  prints local timestamps everywhere, but a local-time id is ambiguous across a
  DST fold, which breaks the one property the id exists for. Randomness comes
  from `getrandom` 0.4 — the version already resolved in the lock via `pesde`'s
  `gix`/`tempfile`/`uuid` stack — declared explicitly in the workspace table
  with its version pinned rather than left implicit. Collisions (same second,
  same suffix) re-mint instead of merging into a stranger's directory. The
  ordering guarantee is at timestamp granularity: ids starting in different
  seconds sort in start order, but two runs starting within one second share a
  prefix and their suffix order is unspecified — a random suffix neither can
  nor should encode start order.

- **The project root is the nearest ancestor containing `.ptah/`.** Not
  `find_project_config` (nearest `.ptah/config.toml`), which would silently fall
  back to the invocation directory in a project that has only `pesde.toml` —
  a project `ptah-pesde` legitimately recognises. Not the invocation directory
  always, which would scatter records into subdirectories. This matches the
  definition `package-management` already uses for the root `.luaurc` ("the
  directory containing `.ptah/`"), so the workspace keeps one notion of project
  root rather than gaining a third.

- **A dedicated `SessionReady` event, reversing a recorded non-goal.** The
  archived `expose-acp-session-id` change explicitly rejected a structured
  readiness payload: "a `SessionReady` variant would serve only a hypothetical
  TUI adapter and is that future change's business". That premise is now false —
  a concrete sink needs the id. Rather than parse `Lifecycle { message }`, ptah
  adds `SessionEvent::SessionReady { label, agent, command, args, env_keys,
  acp_id }`, following the `ToolLine` precedent (the structured facts travel
  beside the rendering decision, and the renderer formats the line). It is
  emitted from `crates/ptah-luau/src/bindings.rs` once `start_session` returns,
  because that is the only site holding all four facts at once: the label and
  agent name are the script's, the ACP id arrives on the returned handle, and
  the authored shape is read there. The ACP driver therefore stops emitting its
  `session ready` lifecycle line (it keeps its config/teardown lifecycle
  emissions); the renderer reconstructs the byte-identical line from
  `SessionReady` in verbose mode, and the record sink reads the facts. This also
  dissolves the tempting-but-wrong change at `bindings.rs:340`, which is the
  `spawning agent` line and has no ACP id yet. Alternatives rejected: adding
  optional structure to the many-purpose `Lifecycle` variant (an audit fact
  would sit in the same event as config and teardown notes), and having the
  record sink parse the rendered message (an audit artifact depending on
  wording the `render-logging` capability is free to change).

- **The authored shape is read through a `Registry::raw` accessor.** `Registry`
  stores specs raw and resolves `${VAR}` only in `resolve`/`resolve_with`, so the
  record's pre-interpolation `command`/`args` are reachable only through a new
  accessor. This keeps the secrets boundary in one place (the config model owns
  what "raw" means) instead of teaching the record to un-interpolate. An inline
  `ptah.agent({ ... })` spec has no registry entry, so the bindings capture the
  authored table values before `.interpolate`; the recorded agent name is the
  registry name for named agents and the authored command for an inline spec.

- **The run-start line is emitted by the composition root onto both renderers.**
  It is not a session event, so the `EventSink` fan-out cannot carry it: the
  root keeps the terminal and record `Renderer` handles, emits the line through
  the `ptah`-attributed method (the `exec_line` gate) on each after the record
  exists, and only then hands the fan-out to the runtime. The terminal
  renderer's `--quiet` gate suppresses it there; the record renderer
  (`quiet: false`) always writes it.

- **The record assigns ask ordinals from its own counter.** `AskRequested`
  carries no ordinal (the renderer's `ask {n}` label does), and asks are
  serialized in issue order, so counting `AskRequested` in emission order yields
  the same ordinal the label shows without parsing rendered wording.

- **`status` is derived from how the process ended, not from script intent.**
  `ok` (exit 0), `failed` (non-zero exit), `cancelled` (the runtime reports a
  terminating signal), `running` (no end yet). A deliberate `ptah.exit(3)` is therefore `failed`, with exit code 3.
  Alternative rejected: an `explicit-exit` status — the CLI cannot distinguish
  "the script finished having decided to fail" from "a task error was never
  observed" (both are exit 1 with different causes), and inventing a status the
  runtime cannot compute reliably is worse than an honest non-zero. The record
  reports outcomes; judging them is the operator's job. `running` on a record
  whose process is gone is the honest signal for "died without teardown".

- **The runtime reports cancellation separately from the exit code.** `RunOutcome`
  carries `code`, `error`, and `undelivered_errors` and nothing else, so a signal
  (130/143) is indistinguishable from a script that chose `ptah.exit(130)`. The
  record needs that distinction, so `RunOutcome` gains `cancelled: bool`
  (`crates/ptah-luau/src/state.rs`), set `true` by the cancel arms in `run.rs`
  (the `End::Cancelled` return and each `shutdown_*` return) and `false`
  everywhere else. `run.json`'s `status` derives from the pair: `cancelled` when
  the flag is set, else `ok`/`failed` from the exit code. A richer `RunEnd` enum
  was rejected as unnecessary — every non-cancel status is already derivable
  from `code` plus `error`/`undelivered_errors`, and classifying setup/read
  failures into new variants would change no observable output. The force-kill
  paths — the signal monitor's second signal (`std::process::exit`) and SIGKILL —
  do not return through `run`, so they leave the record at `running`, the same
  honest "died without teardown" state the `status` decision already defines.

- **`run.json` records invocation *shape*, never secrets.** `command` and
  `args` are taken from the configuration layer, before `${VAR}` interpolation,
  and env contributes key names only, never values. The proposal's source material says
  "the resolved registry's command/arg shape"; *resolved* is the wrong word for
  a security boundary, because `${VAR}` interpolation is a supported feature and
  `args = ["--key", "${ANTHROPIC_API_KEY}"]` is a documented pattern — a
  resolved arg list can literally contain an API key. What is worth recording is
  a property of the configuration file, read through `Registry::raw`. Only agents
  whose sessions actually started appear, so an unused registry is neither leaked
  nor implied to have run.

- **Recording is best-effort; the directory and its `.gitignore` are one
  unit.** Attempt to create them together; if either fails, skip the record,
  warn once on stderr, and run. Never create a record without its ignore file —
  a record that can leak into a commit is worse than no record, since `.ptah/`
  is legitimately tracked and workflows `git add -A` as routine. Alternatives
  rejected: hard-failing the run (makes ptah unusable in read-only checkouts,
  which the deferred override means there is no way out of), and creating the
  directory without the ignore file (silent secret-leak hazard). Mid-run write
  failures disable the record and never abort the run. The warning prints even
  under `--quiet`, like ask lines: it reports a missing artifact, and silence
  there is the failure mode this whole change exists to remove.

- **The log is the renderer's stream, so script `print` is not in it.** The
  log therefore cannot stand in for a stdout redirect for scripts that print.
  Alternative rejected: capturing process stdout at the fd level, which would
  have made the log complete but introduces platform-specific fd surgery and
  races the renderer's own stdout. Scripts wanting a durable record should use
  `ptah.log`, which is on the sink and therefore in both.

- **`run.json` is rewritten on ask *request* as well as resolution.** An ask the
  run was torn down waiting on is exactly the state an operator wants to see,
  and the request event already carries the prompt. Cost: one more small atomic
  rewrite.

## Risks / Trade-offs

- [The record is unpinned traceability — no snapshot, no hash, no git state] →
  Accepted and named in Non-Goals. The record answers "when, which script,
  which agents, how it ended". If a run's code provenance matters, that is the
  first follow-up, not a silent gap.
- [Two renderers can disagree on a line's timestamp by up to a second across a
  boundary] → Accepted; the file is an audit artifact, not a byte-for-byte
  mirror. If byte-identity is ever wanted, it is exactly the mirroring-writer
  alternative above.
- [A record's `log` is not a stdout capture, so `ptah run … > out.log` and the
  record are not interchangeable] → Documented in the README section; scripts
  that need their own output recorded use `ptah.log`.
- [`run.json` becomes the most secret-bearing file ptah writes — prompts, agent
  output, and the operator's own ask answers] → The self-ignoring
  `.gitignore` is the control, created atomically with the directory; the
  README's security note must name `run.json` explicitly rather than only
  "logs may contain tokens".
- [The structured readiness payload reverses a decision recorded in an archived
  change] → Recorded here with the reason the original premise no longer holds,
  so the reversal reads as deliberate rather than accidental drift.
- [A SIGKILL between the record directory's creation and the first `run.json`
  write leaves an empty record directory] → Acceptable: the directory exists
  with its ignore file and no metadata; the operator sees an anomalous entry
  rather than a leak or a half-written file.
- [`getrandom` becomes an explicit workspace dependency] → Already resolved in
  the lock at 0.4.3; the pinned declaration makes the edge honest instead of
  relying on a dependency that could vanish from the lock.
- [A second SIGINT/SIGTERM hard-exits before the final `run.json` write] →
  The record is left at status `running`, exactly like a SIGKILL. Accepted: the
  hard exit exists precisely because teardown is wedged, and an anomalous
  `running` record is honest where a fabricated `cancelled` would not be.

## Migration Plan

Additive only. No config, no wire changes, no CLI surface, and no change to any
existing script's behavior. Rolling back is reverting the commit; the only
trace is left-over `.ptah/runs/` directories, which are ignored by their own
`.gitignore` and safe to delete. Scripts are unaffected in both directions
because the change adds no script-visible API.
