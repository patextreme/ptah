# Design: expose-acp-session-id

## Context

The ACP session id is born in `session/new`'s response: `Handshake.session_id`
(`crates/ptah-acp/src/proto.rs`) holds it, and the driver (`driver.rs`) uses it
internally for routing (`session/prompt`, cancel, set-config) — but the ready
oneshot carries `Ok(())`, so the id never crosses to `SessionHandle`
(`crates/ptah-core/src/session.rs`) and stays invisible to both the renderer
and the Luau session object. Session creation is already synchronous with the
handshake: `session()` awaits `start_session` through ready, so any id the
ready channel carries exists before a script can hold the session object.

Upstream facts that shaped decisions: ACP v1's `NewSessionRequest` has no
client-supplied session id — the agent generates it. ptah-core already imports
ACP schema types (`ToolKind` in `events.rs`), so carrying a session id through
core raises no adapter-freedom concern.

## Goals / Non-Goals

**Goals:**
- One plumbing path: ready channel → `SessionHandle` → (Luau method, rendered
  line), fed from a single capture point in the driver.
- Zero behavior change anywhere else: no wire traffic, no ask changes, no new
  event variants.

**Non-Goals:**
- `session/load` / `session/resume` (protocol reattach surfaces) — ptah does
  not drive them; exposing the id is merely the prerequisite for any future
  resume story.
- Transcripts or on-disk session logs.
- Structured (machine-readable) exposure of the id on `SessionEvent` — the
  script API covers machine access; a `SessionReady` variant would serve only
  a hypothetical TUI adapter and is that future change's business.
- Auto-injecting the id into `ptah.ask` details.

## Decisions

- **Method name `sessionId()`** (settled over `acpId()`/`agentSessionId()`):
  follows ACP v1's own session-setup field name, so scripts read the same word
  the protocol and agent tooling use. Distinctness from `label()` is pinned in
  the scripting delta. It is a method, not a property, matching the existing
  `label()` convention.
- **Non-optional `string`** (settled over `string?`): ptah speaks only ACP —
  the sole `AgentTransport` impl and the mock are both ACP — so every session
  has an id and `nil` branches would be dead code in every consumer. The port
  contract is documented as "the agent-assigned ACP session id"; if a
  non-ACP transport is ever funded (its own design decision per AGENTS.md),
  optionality gets revisited then.
- **Rust field `SessionHandle.session_id: String`**: matches the
  `agent_client_protocol` field name and the script-facing method. Alternatives
  (`acp_session_id`) rejected — it would be the one place in the stack not
  matching the schema's own name; the doc comment pins the meaning.
- **Plumbing: the ready oneshot payload becomes `Result<SessionId, _>`**
  (currently `Result<(), _>`). Single capture point: the driver already holds
  `hs.session_id` where it emits "session ready" and signals ready — both
  consumers (line format, handle field) read it there. Alternative (a shared
  `Arc<Mutex<Option<String>>>` filled by the driver) rejected: it introduces a
  filled-later state the current sequence doesn't have — ready is signaled
  strictly after the id exists.
- **Render: extend the existing "session ready" `Lifecycle` message** to
  `{label}: session ready (acp {id})` rather than a new event or line. The line
  already exists, is verbose-gated (default `Renderer::lifecycle` suppresses
  non-verbose/quiet), and `Lifecycle`'s doc claims session readiness as its
  domain. The `acp` marker is deliberately not `sessionId`-named: rendered
  output names the ecosystem whose tooling accepts the id; the API follows the
  protocol. Mixed naming is intentional, recorded here.
- **No `agent-sessions` delta**: the ACP client's wire behavior is unchanged
  (ptah merely keeps a value it already receives). The observable behavior
  lands in `scripting` (API) and `render-logging` (line) — spec'd there.
- **`label()` gets spec'd in the same scripting requirement** (it never had a
  scripting requirement of its own): the label/sessionId distinction is the
  heart of the issue, and one requirement holding both makes the contrast
  normative instead of incidental.

## Risks / Trade-offs

- [Field on `SessionHandle` is public facade surface — a permanent API
  commitment] → Doc comment states the contract (agent-assigned ACP session
  id, stable for the session's lifetime); the `AgentTransport` port doc gains
  the same note so future transport impls know they must supply one.
- [Agent-generated ids are opaque and agent-specific — a "session id" from one
  agent may not be resume-able in another's tooling] → Documented as opaque:
  ptah only surfaces it; consumers correlate with the agent that produced it
  (the label names that agent).
- [Definitions byte-identity (`ptah types` gate) breaks if `.ptah/ptah.d.luau`
  and the embedded source drift] → The in-repo file *is* the embedded source;
  the existing gates (types test, StyLua exemption) catch drift; the probe
  script extends to exercise `sessionId` per the synchronized-definitions
  requirement.
- [Verbose-line format change breaks parsers keyed on the old wording] → The
  line is verbose-only operator diagnostics (not a stable machine surface);
  acceptable by design, and the delta spec pins the new shape going forward.

## Migration Plan

Additive only: a new method, a wider verbose line, extended definitions and
docs. No removals, no config, no wire changes — rolling back is reverting the
commit; scripts written against `sessionId()` are the only dependents and fail
with a clear nil-call error on older binaries.
