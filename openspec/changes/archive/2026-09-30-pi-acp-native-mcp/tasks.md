# Tasks: pi-acp native MCP delivery

Development happens in the `.work/pi-acp` clone (gitignored) against pinned rev
`b0581c9`; the diff is exported to
`nix/packages/pi-acp/mcp-config.patch`. Design reference: `design.md` (D1–D6).

## 1. pi-acp patch (in `.work/pi-acp`)

- [x] 1.1 In `src/acp/mcp-config.ts`, add the native config writer
  (`exposure: 'direct'`), the generated per-session extension writer, and the
  cached `pi --help` capability probe (priority: `--mcp-config` → adapter,
  `pi mcp` → native, else none) returning a delivery descriptor; verify unit
  tests for filtering, both config writers, and config file mode 0600 pass via
  `npm test`
- [x] 1.2 Verify a unit test imports the generated `.mjs` extension with a
  stub `pi` object and asserts one `registerMcpServer` call per server with
  `exposure: 'direct'`, and that it no-ops when `PI_ACP_MCP_CONFIG` is unset
  or `pi.registerMcpServer` is missing
- [x] 1.3 Thread the delivery descriptor through `src/acp/agent.ts`,
  `src/acp/session.ts`, and `PiRpcProcess.spawn` (adapter → `--mcp-config`;
  native → `-e <ext>` + `PI_ACP_MCP_CONFIG`, no other child-env change) with
  dispose cleanup; verify component tests assert argv and child env for both
  deliveries and temp-dir removal after session end
- [x] 1.4 Extend `test/helpers/fake-pi.mjs` with a native-MCP `--help` switch
  and child-env recording, update `test/unit/session-restore.test.ts`; verify
  `npm test`, `npm run lint`, and `npm run typecheck` are all green

## 2. Patch export and flake package

- [x] 2.1 Export the diff over the touched files to
  `nix/packages/pi-acp/mcp-config.patch` and verify `git apply --check` is
  clean against rev `b0581c9`
- [x] 2.2 Update `nix/packages/pi-acp/README.md` (native MCP primary, adapter
  a fallback for older pi, probe semantics, pi ≥ 0.99 requirement) and verify
  every documented command matches the shipped behavior
- [x] 2.3 Run `nix build .#pi-acp` and verify the output contains
  `dist/index.js` with the native delivery wiring and that the build succeeds

## 3. Integration verification

- [x] 3.1 End-to-end against the real, adapter-free pi 0.99.1 from
  `nix develop`: a `resultSchema` script run via `ptah run --agent pi` returns
  a typed result (not `nil`); if model credentials are unavailable, verify
  instead that spawning pi through the patched adapter connects the `ptah`
  bridge and `ptah_result_submit` is reachable
- [x] 3.2 Degradation check: point `PI_ACP_PI_COMMAND` at a fake pi whose
  `--help` advertises neither capability and verify the run completes with the
  warning and neither `-e` nor `--mcp-config` passed
- [x] 3.3 Run `openspec validate --change pi-acp-native-mcp` (passes) and the
  change-relevant build gates: `nix build .#pi-acp` and
  `checks.ptah-{check,analyze,smoke}` all pass. Full `nix flake check` still
  fails `checks.ptah-tests`, but that is pre-existing and unrelated (see the
  note below); tracked as [#41](https://github.com/patextreme/ptah/issues/41)

## Pre-existing, unrelated failure (not this change)

`checks.ptah-tests` fails 10 `ptah_libs` component tests on the pristine tree
at `5b3e9e5` with this change's edits stashed (e.g.
`mistyped_component_config_is_a_check_finding`,
`openspec_component_implements_an_unresolvable_scope_fails`). The pi-acp
delivery change touches no Rust code and adds no failures: 23 tests pass, the
10 failures reproduce identically without it. Tracked as
[#41](https://github.com/patextreme/ptah/issues/41).
