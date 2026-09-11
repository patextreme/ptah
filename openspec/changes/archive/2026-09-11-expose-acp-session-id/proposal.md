# Proposal: expose-acp-session-id

## Why

The agent-generated ACP session id is internal to ptah: scripts only see the
ptah-local label (`agentName/localId`), which maps to nothing in the agent's
own tooling. A human who needs to inspect a headless session — most importantly
one paused on a `ptah.ask` — has no way to correlate it with the agent's
session tooling (e.g. `claude --resume <sessionId>`), because ptah renders no
transcripts and has no resume mechanism of its own (issue #20).

## What Changes

- **Session API**: session objects gain `session:sessionId() -> string`
  returning the agent-assigned ACP session id (the id the agent generated in
  its `session/new` response). Non-optional: ptah speaks only ACP, so every
  session has one. It is a method (like `label()`), and clearly distinct from
  `label()`, which stays the human-facing attribution id in rendered output.
- **Rendered output**: the existing verbose-only lifecycle line
  `{label}: session ready` becomes `{label}: session ready (acp {id})`, so
  operators get post-hoc correlation with agent-side logs without any script
  changes. The `acp` marker is deliberate (names the ecosystem whose tooling
  accepts the id), even though the API method follows the protocol's own name.
- No changes to `ptah.ask` (scripts compose the id into ask details
  themselves), no new structured `SessionEvent` variant (the ready line stays
  a formatted `Lifecycle` message, per existing pattern), no ACP wire-level
  changes.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `scripting`: add a requirement pinning session identity methods —
  `label()` (existing behavior, first spec'd here) and the new `sessionId()`
  with its distinction from the local label.
- `render-logging`: add a requirement for the session-ready lifecycle line
  carrying the ACP session id, gated to verbose output (lifecycle lines were
  not previously pinned in the spec).
- `type-definitions`: the definitions-cover-the-API requirement's session
  object method list gains `sessionId`.

## Impact

- `crates/ptah-acp` — the ready oneshot carries `Ok(session_id)` instead of
  `Ok(())`; the "session ready" lifecycle line format gains the id;
  `SessionHandle` construction passes it through.
- `crates/ptah-core` — `SessionHandle` gains a `session_id: String` field
  (public facade surface; named after the protocol's own field).
- `crates/ptah-luau` — `new_session_obj` builds the `sessionId` method.
- `.ptah/ptah.d.luau` — `Session` type gains `sessionId: (self: Session) -> string`
  (the file is the embedded definitions source; `ptah types` byte-identity is
  the gate).
- `crates/ptah-cli/tests/fixtures/types_probe.luau` — probe exercises the new
  member (required by the definitions-synchronized requirement).
- `README.md` and `skills/ptah/SKILL.md` — session-method tables gain a row;
  skill doc gains a short `sessionId()` → `ptah.ask` details example.
- Tests: ptah-luau unit coverage for the method; e2e assertion that the
  verbose ready line carries the mock agent's id.
- Not affected: `agent-sessions` (the ACP client's wire behavior is
  unchanged — the id is merely captured, not newly requested), `ask`
  (deliberately untouched), shell-exec, typed-results.
