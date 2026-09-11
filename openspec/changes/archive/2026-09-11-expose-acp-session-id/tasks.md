# Tasks: expose-acp-session-id

## 1. Core plumbing (id crosses the ready boundary)

- [x] 1.1 Add `session_id: String` to `SessionHandle` (`crates/ptah-core/src/session.rs`) with a doc comment pinning the contract (agent-assigned ACP session id, stable for the session's lifetime); extend the `Debug` impl. Mirror the contract note in the `AgentTransport::start_session` port doc (`crates/ptah-core/src/ports.rs`). Verify: `cargo build -p ptah-core` compiles and `cargo test -p ptah-core` passes.
- [x] 1.2 Change the ready oneshot payload in `crates/ptah-acp/src/driver.rs` from `Result<(), _>` to `Result<SessionId, _>`, sending `hs.session_id` where `Ok(())` is sent today; construct `SessionHandle` with the received id. Verify: `cargo build -p ptah-acp` compiles; existing ptah-acp tests pass (`cargo test -p ptah-acp`).

## 2. Scripting API

- [x] 2.1 Build the `sessionId` method on the session table in `new_session_obj` (`crates/ptah-luau/src/bindings.rs`), modeled on the existing `label` method. Verify: a ptah-luau test creates a session against the loopback/mock transport and asserts `sessionId()` returns a non-empty string distinct from `label()` (scripting delta scenarios).
- [x] 2.2 Add `sessionId: (self: Session) -> string` to the `Session` type in `.ptah/ptah.d.luau` with a comment distinguishing it from `label` (the agent-side ACP id vs the local attribution label). Verify: `cargo test --test types` passes (byte-identity with `ptah types` output) and `ptah check` on the definitions-bearing fixtures stays clean.
- [x] 2.3 Extend the runtime probe (`crates/ptah-cli/tests/fixtures/types_probe.luau`) to call `session:sessionId()` and use the result as a string, keeping every member exercised. Verify: probe test passes (`cargo test --test types` or its owning suite).

## 3. Rendered output

- [x] 3.1 Change the session-ready lifecycle message in `crates/ptah-acp/src/driver.rs` to `{label}: session ready (acp {id})`. Verify: an e2e test (`crates/ptah-cli/tests/`, mock agent, `--verbose`) asserts the rendered line carries the mock's session id, and that default/quiet runs show no ready line (render-logging delta scenarios).

## 4. Documentation

- [x] 4.1 Add `session:sessionId()` rows to the session-method tables in `README.md` and `skills/ptah/SKILL.md`, and add the short skill-doc example composing `sessionId()` into `ptah.ask` details (agent-agnostic wording; no canonical key convention). Verify: docs render sensibly; no spec contradicts them.
- [x] 4.2 Update the skill-doc trigger description if it enumerates session methods (it lists `session:prompt` today) — decide minimal touch: adding `sessionId` to the enumerated list or leaving the enumeration as a representative sample, consistent with how `label`/`configOptions` are handled there. Verify: grep shows consistent method enumeration across the doc.

## 5. Gates and validation

- [x] 5.1 Run the full local suite: `cargo test` (workspace), `stylua .` (definitions exempt), and `ptah check` on the bundled examples touched by the change (none expected). Verify: all green.
- [x] 5.2 Run `openspec validate expose-acp-session-id --strict` and fix any findings. Then `nix flake check` for the sandbox gates (Luau format + in-place check with the release binary). Verify: both pass.
