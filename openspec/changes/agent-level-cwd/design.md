# Design: agent-level-cwd

## Context

See `proposal.md` — Why. The current state that shapes the approach:

- **The session cwd rides the ACP wire, not the process spawn.** The agent
  subprocess is spawned via the ACP library without a working-directory
  override (`crates/ptah-acp/src/process.rs` — `AcpAgent::spawn_process`); the
  session's cwd is conveyed to the agent in the `session/new` request
  (`crates/ptah-acp/src/proto.rs`, `NewSessionRequest::new(cwd)`), flowed from
  `SessionOptions.cwd` (`ptah-core/src/session.rs`). An agent-level cwd is
  therefore purely a *defaulting* change upstream of the transport — no
  protocol work.
- **`AgentSpec` is the single shape for both sources.** Registry entries
  deserialize into `ptah_core::config::AgentSpec` (`crates/ptah-core/src/config/mod.rs`);
  the inline `ptah.agent({ … })` table is parsed into the same struct
  (`crates/ptah-luau/src/bindings.rs`, alongside an untouched *authored* copy
  kept for attribution). Both sources gain the field through one struct.
- **Per-session cwd resolution lives in the Lua binding.**
  `bindings.rs` (`new_agent_factory`'s `session` closure) resolves
  `opts.cwd` (absolute kept, relative joined to `state.invocation_dir`) and
  defaults to `invocation_dir` — the exact seam where handle defaulting slots
  in as the middle tier.
- **Nothing validates cwd today.** A bogus per-session cwd reaches the agent
  raw; the agent fails (or misbehaves) downstream. The mock agent already
  echoes the session cwd (`MOCK_ECHO_CWD`, "for default-cwd tests"), so
  offline assertions need no mock changes.
- **The definitions file is the source of truth.** `.ptah/ptah.d.luau` is
  committed, exempt from StyLua, and byte-identical to `ptah types` output;
  `ptah-check` embeds it for analysis. Editing it in place keeps every
  consumer in sync with no generation step.

## Goals / Non-Goals

**Goals:**

- One optional `cwd` on `AgentSpec` serving registry entries and inline specs.
- Precedence: session option → handle cwd → invocation directory, resolved per session.
- Fail-fast validation with a catchable, directory-naming Lua error at `session()`.
- Zero change to the ACP layer, `ptah.exec`, env handling, and teardown.

**Non-Goals:**

- No worktree lifecycle (creation/removal) — callers drive `git worktree`.
- No per-session `env` override and no change to environment inheritance.
- No `ptah.exec` cwd option (README keeps its "no `cwd`/`env` override in v1" line).
- No run-record/readiness-event schema growth (see Decisions).

## Decisions

- **`cwd: Option<String>` with `#[serde(default)]` on `AgentSpec`, interpolated
  in `AgentSpec::interpolate`.** TOML parsing, project/user merge, and
  `${VAR}` interpolation come from the existing derive + interpolation walk —
  `ptah-config` needs no code change. Empty-string expansion of an unset
  `${VAR}` in `cwd` is *not* special-cased: it fails the directory validation
  like any unusable path, consistent with how an empty interpolated `command`
  fails at spawn. (Alternative: resolve `${VAR}` to an error on unset —
  rejected; it breaks the documented unset→empty contract for every field.)

- **Validation happens in the `agent:session` binding, on the final resolved
  cwd, before `transport.start_session`.** The binding is the only site that
  holds the label, the invocation dir, and the resolved handle spec together,
  the error must be a catchable Lua error at `session()`, and ptah-acp is a
  transport that never touches the filesystem. (Alternative: validate in
  `ptah-acp` at spawn — rejected on both layering and mechanics: the cwd never
  affects the process spawn.)

- **Validation is uniform across cwd sources — a deliberate behavior change.**
  Today an explicitly wrong per-session `cwd` reaches the agent unvalidated.
  Failing fast at `session()` for every source gives one mental model and
  matches the issue's acceptance criteria ("a Lua error naming the directory").
  A script that passed a not-yet-created directory was broken anyway; the new
  error just names the real culprit instead of leaving the agent to fail
  obscurely. The delta spec pins this, so the behavior change is spec-visible.

- **The readiness event and run record stay unchanged.** They carry the
  authored command/args/env-*keys*; authored `cwd` does not join. Rationale:
  no attribution need (cwd is not consumed by the record today), and keeping
  the event shape stable avoids record-format churn. Revisit if a consumer
  asks for cwd in the record — that is a record change, not part of this one.

- **Validation is a courtesy check, not a guarantee.** Between `session()`'s
  `exists`/`is_dir` probe and the agent actually using the directory, the path
  can vanish (worktree removed mid-run). Ptah validates once at session
  creation; after that the directory is the agent's concern. Documented as a
  risk below rather than gold-plated with watches.

- **Struct-literal churn is accepted, not abstracted.** Adding a field breaks
  the handful of `AgentSpec` literals (bindings' authored copy, core config
  tests, the acp test helper, the render record test). They update
  mechanically; `AgentSpec::new` keeps its command-only shape with the field
  defaulting to `None`. A builder is not warranted for a three-field struct.

## Risks / Trade-offs

- [TOCTOU: directory vanishes after validation] → Accepted; the check exists
  to fail fast with a clear message, not to police the filesystem. The agent
  surfaces its own cwd errors for anything later.
- [Scripts relying on permissive bogus-cwd pass-through] → Spec-visible
  behavior change, called out in the proposal; the error is catchable and
  names the directory, so affected scripts get an actionable message.
- [Users expect the agent *process* to run in `cwd`, not just the session] →
  README documents the actual semantics (cwd reaches the agent as the
  session's working directory via `session/new`; the subprocess itself spawns
  in ptah's cwd) — the same semantics the per-session option always had.

## Migration Plan

Purely additive; no migration. Registry files and inline specs without `cwd`
parse and behave exactly as today; rollback is revert.

## Open Questions

None.
