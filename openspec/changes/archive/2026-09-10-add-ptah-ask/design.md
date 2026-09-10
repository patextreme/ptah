# Design: add-ptah-ask

## Context

The runtime already has every mechanical ingredient this change needs; the
design is assembly, not invention:

- **Blocking bindings suspend one coroutine** via mlua async closures
  (`session:prompt` parks on a oneshot; `ptah.exec` parks on the port future
  raced against the shutdown watch — `bindings.rs` is the template).
- **Injected capabilities** flow through `RunConfig` into `RuntimeState`
  (`process_runner: Option<Arc<dyn ProcessRunner>>` with its clear
  "no runner injected" error). The port set is closed by policy; this change
  is the deliberate change that funds port #6, like `add-shell-exec` was for
  #5.
- **The renderer owns stdout exclusively** (mutexed `BufWriter`, flush per
  line); nothing in the host reads stdin today. All display therefore flows
  through the `EventSink` port; the stdin provider never writes to stdout.
- **Config is strictly per-agent today**; `[ask]` is the registry's first
  global data, so the parse/merge/model layers all widen one notch.
- **Check lints walk the literal require graph** with a full-moon
  `Collector` that already matches `ptah.agent` member calls; an ask branch
  is a variation of it.

See proposal.md for motivation; the specs carry the behavior contract.

## Goals / Non-Goals

**Goals:**

- A `ptah.ask` whose suspension is mechanically indistinguishable from
  `ptah.exec`'s (per-coroutine, teardown-dropped, event-observed).
- Provider selection resolved exactly once at the composition root, consumed
  everywhere (run binding, preflight, check) as a plain value.
- Keep core I/O-free (deps_guard) and keep every new I/O touchpoint (isatty,
  stdin) in `ptah-cli`.

**Non-Goals:**

- Durable suspension / restart recovery; ACP `session/resume`.
- Additional providers (webhook, Slack, TUI) and provider settings
  subtables — they extend the same port and `[ask]` schema later.
- Answer `choices`, `multiline`, `timeoutMs` options; ACP elicitation
  passthrough (orthogonal: today every non-permission agent→client request
  gets method-not-found; wiring elicitation to the ask port would be its own
  change).
- Warning-severity findings in `ptah check`. The ask finding is exit-1 like
  every other in-process finding; the "unresolvable" case fails rather than
  warns by decision (issue grilling, Q8c).

## Decisions

### D1 — New port `AskProvider`, injected like `ProcessRunner`

`ptah-core/src/ports.rs` gains:

```rust
pub trait AskProvider: Send + Sync {
    fn ask<'a>(&'a self, request: AskRequest)
        -> Pin<Box<dyn Future<Output = Result<AskOutcome, AskError>> + Send + 'a>>;
}
```

with `AskRequest { prompt, details, attribution }`,
`AskOutcome { Respond { text } | Abort }`, and `AskError { InputClosed |
Failed(String) }` — enough for the binding to produce the four stable error
messages without knowing the provider. `Prohibited` and `no-provider` are
*mode* states, not provider results; they raise before the port is called.

**Alternatives:** extending `InteractionPolicy` (rejected: it answers
agent→client permission requests hardlessly and is hard-coded inside the ACP
driver; `none` must not tangle with the allow-all posture, and the driver is
the wrong layer for script-issued questions); no trait, a concrete type in
`RunConfig` (rejected: kills the provider abstraction the issue is about and
breaks the fake-provider test seam).

### D2 — Three-state interaction mode resolved once, at the composition root

```rust
enum InteractionMode { Provider(Arc<dyn AskProvider>), Prohibited, Unresolved }
```

`cli.rs` resolves it once: `--ask` (clap-validated) → `PTAH_ASK` (validated;
invalid value is a config failure → stderr + exit 2, matching registry
discovery failures) → `[ask]` from the merged registry (validated at parse)
→ auto-detect (`libc::isatty` on stdin **and** stdout → stdin provider) →
`Unresolved`. The resolved mode feeds two consumers: the check/preflight
lint (findings only when ask call sites exist) and `RunConfig.interaction`
for the binding. Non-TTY is `Unresolved`, never implicit `none` — the two
error messages must not lie about which problem the operator has.

The known-provider value set lives once in core
(`ptah-core/src/config`: an `AskProviderKind { Stdin, None }` enum behind
`FromStr`), consumed by the config parser, the `PTAH_ASK` read, and the
clap value parser. Unknown keys inside `[ask]` stay tolerated (the registry
parses without `deny_unknown_fields` everywhere; consistency over
strictness in v1).

**Alternative rejected:** resolving lazily at first `ptah.ask` — preflight
must see the resolution before agents spawn.

### D3 — The binding follows the `exec` template; serialization lives in the binding

`bindings.rs` gains `ask` as an async closure: validate `opts` (usage error
for missing/non-string `prompt`), take the per-run ask counter + attribution
label (`ask {n} {script_basename}`, from `RuntimeState.script_path`), match
the mode (`Prohibited`/`Unresolved` raise immediately), then — holding a
dedicated `tokio::sync::Mutex` ask lock taken *before* emitting — emit
`AskRequested` through the sink, call the provider, emit `AskResolved`, and
return the result table. Teardown drops the parked future like every other
blocking call (no resolve event, no abort delivery).

Concurrent asks serialize **at the binding layer**, not in the port: the
lock is taken in FIFO issue order, so every provider — including test fakes
injected via `RunConfig` — gets exactly one ask at a time, and the FIFO
scenario is testable without the real stdin provider. The port contract
stays per-request; a future provider that could safely parallelize would
need a spec change, not just an impl.

**Alternative rejected:** a serializing wrapper composed around the provider
at injection time — then in-process tests with a bare fake provider would
bypass serialization and the guarantee would be untested.

### D4 — Display is event-driven; the stdin provider only reads

The stdin provider never writes to stdout (the renderer owns it). The
binding's `AskRequested` event carries prompt, details, and the attribution
label; `ptah-render` renders the prompt line, the indented details line,
and a `> ` input cue (written without a trailing newline and flushed — the
user's own Enter terminates the visual line; over pipes the next rendered
line simply follows). `AskResolved` renders one action line (`respond` /
`abort`) — the answer text rides the event for downstream sinks but is
never re-echoed. Ask lines bypass `--quiet` (gated like `script_log`, not
like session events) because a suppressed prompt is a hung run.

`emit` is synchronous, so the prompt is on screen (flushed) before the
provider first polls stdin — no ordering race.

The stdin provider reads ptah's real stdin via tokio's async line reader
(shared handle constructed once at injection), resolves `/abort` and — when
stdin is a TTY — Ctrl-D-on-empty to `Abort`, and maps EOF on non-TTY stdin
to `AskError::InputClosed` (the distinct end-of-input message). This
TTY/pipe distinction is what makes Ctrl-D an abort gesture interactively
while a closed pipe in scripted runs is an error rather than a phantom
abort.

**Alternative rejected:** the provider rendering its own prompt — would
need the stdout mutex and duplicate the renderer's truncation/timestamp/
quiet logic; future providers would each reimplement display.

### D5 — Registry widening is additive and wholesale-merged

`RegistryFile` gains `ask: Option<AskSection>`; `parse_layer` returns the
pair; `Registry` carries `ask: Option<AskSection>`; `from_layers` applies
`project.ask.or(user.ask)` — whole-section replacement, mirroring the
per-agent-name rule. `AgentSpec`/interpolation/`resolve` are untouched (the
section holds a validated enum, nothing to interpolate; credentials-in-file
is structurally impossible in v1 because there is nothing to configure
beyond the provider name). Validation errors label the offending
**layer** (`user`/`project`), not the file path — the established
`ConfigError` granularity shared by every other config error; with
exactly two possible files the layer is equally diagnostic.

### D6 — The ask lint mirrors the agent-name lint, with the call site as the signal

`Collector` gains an `ask` branch (same shape as the `ptah.agent` branch:
`Prefix::Name("ptah")` + `Dot "ask"` + `Call`, but with **no** literal-arg
restriction — the call itself is the capability signal). `check()` and
`preflight()` receive the resolved `InteractionMode` (the CLI owns
terminal detection; ptah-check stays free of process context) and emit:

- ask sites present + `Prohibited` → "interaction is prohibited (`none`)…"
- ask sites present + `Unresolved` → "no ask provider resolvable — pass
  `--ask`, set `PTAH_ASK`, or configure `[ask]`"
- no ask sites → nothing, whatever the mode.

Alias-indirected asks (`local f = ptah.ask`) are not collected — same
documented-residual class as computed agent names; the runtime check is the
source of truth.

### D7 — Events, types, docs

- `SessionEvent` gains `AskRequested { prompt, details }` and
  `AskResolved { action, text: Option<String> }` (data only; core stays
  I/O-free, deps_guard untouched).
- `.ptah/ptah.d.luau`: `AskOptions`/`AskResult` types + the `ask` field;
  the runtime probe script gains ask coverage; byte-identity and
  `ptah init` sync guards apply automatically.
- `ptah init` skeleton gains a commented `[ask]` block (still a valid empty
  registry as written); README (namespace table, new "Asking a human"
  section, registry docs, CLI flags, a one-line amendment to the
  headless-permissions section) and `skills/ptah/SKILL.md` (API row,
  semantics bullet, registry + check sections) updated in the same change.

### D8 — Test strategy (offline, per repo rules)

1. **In-process semantics** (`tests/script.rs`-style, fake provider via
   `RunConfig`): respond/abort shapes, usage error, all four raises,
   FIFO serialization of two concurrent asks, ask-at-script-end keeps the
   run alive, teardown drop (no resolve event).
2. **Binary e2e with piped stdin** (new `spawn()`-based harness variant:
   piped stdin+stdout, read lines until the ask prompt renders, write the
   answer): line answer, `/abort`, closed-stdin raise. Explicit
   `--ask=stdin` makes the provider legal without a TTY.
3. **Session survival**: task A in a slow mock turn (`MOCK_DELAY_MS`), task
   B asks and is answered; A completes; the same session serves another
   prompt (unchanged pid asserted via harness process counting).
4. **Check/preflight**: prohibited and unresolvable findings, explicit
   `PTAH_ASK=stdin` clean in CI conditions, no-ask + `none` clean, aliased
   ask not collected.
5. **Examples**: `examples/ask.luau` behind the piped-stdin harness
   variant; negative-path (ask + `none` → exit 1) rides the existing
   `.output()` harness.

## Risks / Trade-offs

- [Renderer/stdout ownership] every ask display path goes through the
  event sink; the provider writes nothing. → Mitigation: D4 keeps a single
  writer; the piped-stdin e2e asserts exact rendered lines.
- [Ctrl-C while parked in a stdin read] the read future is dropped at
  teardown like every other binding. → Mitigation: tokio async stdin (no
  blocking `std` read on the main thread); second-signal force-escape
  contract unchanged.
- [Preflight false positives] an ask on a dead branch fails `check` under
  `none`/CI-unresolvable. → Mitigation: documented, spec'd residual (same
  stance as unreachable-missing-require); the remedy is one env var.
- [Env/config validation drift] three places read provider names. →
  Mitigation: single core enum consumed by all three (D2).
- [`[ask]` on an older ptah] silently ignored (no `deny_unknown_fields`).
  → Accepted: forward-compatible by the same accident the rest of the file
  already relies on.
- [Quiet-mode surprise] `--quiet` still prints ask lines by design.
  → Mitigation: documented in the flag's help and README; it is required
  interaction, and `none` exists precisely for truly unattended runs.

## Migration Plan

Purely additive; no existing script, config, or flag changes behavior.
Rollback is reverting the change; the only persisted artifact it can leave
is a commented `[ask]` block in a user's `config.toml` skeleton, which
older binaries ignore.
