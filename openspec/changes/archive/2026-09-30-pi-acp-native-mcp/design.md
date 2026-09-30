# Design: pi-acp native MCP delivery

## Context

See `proposal.md` — Why. Facts that shape this design, verified against the
installed pi 0.99.1 and pi-mcp-adapter 3.3.0:

- **pi 0.99.0 added built-in MCP.** `pi mcp add|remove|list|login|logout`
  manage servers from `~/.pi/agent/mcp.json` and, in trusted projects,
  `.pi/mcp.json`; extensions add per-session servers with
  `pi.registerMcpServer(name, config)`. `config` has the shape of an
  `mcpServers` entry plus `exposure`, `toolExposure`, `enabled`, `timeout`.
- **`--mcp-config` is not native.** It is registered by `pi-mcp-adapter`
  (`pi.registerFlag('mcp-config', …)`). `pi --help` shows it **only when the
  extension is loaded/installed**; `pi --help` never shows it otherwise, and
  `pi --mcp-config <path>` then fails with `Unknown option`.
- **The two mechanisms are mutually exclusive.** Per pi's docs, an extension
  that registers `/mcp` (pi-mcp-adapter does) *replaces* built-in MCP. So when
  the adapter is installed, native registration would not be authoritative;
  `--mcp-config` should win.
- **`pi.registerMcpServer` is a real runtime path.** A generated extension
  loaded with `-e <file>` that calls it makes built-in MCP connect the server
  (verified: `initialize` → `notifications/initialized` → `tools/list` reach
  the stdio server).
- **A throwing extension factory aborts pi** (`Error: Failed to load
  extension …`, exit 1). Passing `-e` therefore requires a prior capability
  probe; we cannot rely on the extension to fail soft on old pi.
- pi-acp is bundled by `tsup` to a single `dist/index.js` (`files: ["dist"]`),
  so a separately-authored extension file would need build-config changes.

## Goals / Non-Goals

**Goals:**

- `resultSchema` scripts against `pi` produce typed results on a stock,
  adapter-free pi 0.99.x.
- Delivery choice is automatic and degrades to the existing warning path.
- Per-session isolation, 0600 config hygiene, cleanup on dispose, and
  stdio-only filtering are preserved.
- The patch stays small and testable in `.work/pi-acp`, rebased into
  `mcp-config.patch` as before.

**Non-Goals:**

- Upstreaming to `svkozak/pi-acp` (unchanged decision).
- Auto-provisioning pi extensions or editing any user-owned pi config file.
- ACP http/sse server support (still warn + drop).
- Any change to ptah's bridge, driver, registry, or the pinned pi-acp rev.

## Decisions

### D1 — Native delivery: generated per-session extension + `PI_ACP_MCP_CONFIG`

For the native path, pi-acp writes, into the same `mkdtemp()` session dir it
already uses, both the MCP config (mode 0600) and a tiny generated extension
(`register-mcp-extension.mjs`). It then spawns pi with
`-e <dir>/register-mcp-extension.mjs` and env `PI_ACP_MCP_CONFIG=<config>`.
The extension reads the file at load and calls
`pi.registerMcpServer(name, { …config, exposure: 'direct' })` per server.

- Chosen over a build-emitted static extension (`tsup` second entry):
  runtime generation needs no build-config change, works identically from
  source (`tsx` tests) and from bundled dist, and keeps the rebased patch
  self-contained. The generated source is still unit-tested by importing the
  written file against a stub `pi`.
- Chosen over writing `.pi/mcp.json` / `~/.pi/agent/mcp.json`: those are
  user-owned, not per-session, need project trust for the project file, and
  would race concurrent sessions. The generated extension mutates nothing
  user-owned.
- Chosen over a self-contained extension implementing the MCP client: pi's
  `registerMcpServer` makes pi own the connection, so the extension is ~20
  lines instead of a JSON-RPC client.

### D2 — `exposure: 'direct'` replaces adapter `directTools: true`

Native entries carry `exposure: 'direct'` so `ptah_result_submit` is declared
to the model like a built-in tool (the same intent as the adapter's
`directTools: true`). Guidance stays in the tool description; ptah adds no
prompt text.

### D3 — Capability probe selects native vs adapter vs none

One cached `pi --help` invocation, parsed in priority order:

1. `--mcp-config` present → **adapter** (the adapter owns `/mcp`; native
   registration would be overridden).
2. else `pi mcp` subcommand present → **native** (pi ≥ 0.99).
3. else → **none**, warn and drop.

The probe is required (not merely advisory) because a failed `-e` extension
aborts pi; it also keeps the user-facing warning for genuinely unsupported pi.
It stays a single `spawnSync` per pi-acp process, cached, exactly as today.

### D4 — Env var + `-e` threading through spawn

`PiRpcProcess.spawn` accepts a small `mcp` delivery descriptor instead of the
single `mcpConfigPath`:

- `{ kind: 'adapter', configPath }` → append `--mcp-config <configPath>`.
- `{ kind: 'native', configPath, extensionPath }` → append `-e
  <extensionPath>` and set `PI_ACP_MCP_CONFIG=<configPath>` on the child env
  (the child env is `{ …process.env }`; only this key is added).
- absent/none → nothing.

Dispose still removes the whole temp dir via the recorded `configPath`.

### D5 — Config shapes split by delivery

`toMcpConfigFile` keeps producing adapter entries (`directTools: true`); a new
`toNativeMcpConfigFile` produces built-in entries (`exposure: 'direct'`).
Both derive from the same filtered ACP stdio server list, so filtering,
ordering, and env mapping are shared.

### D6 — Test shape

The patch's suite gains: unit tests for filtering, both config writers, the
generated extension (loaded with a stub `pi`), probe caching/priority, and the
warn-and-drop path; component tests asserting the spawn argv and env for both
deliveries and cleanup on dispose. `test/helpers/fake-pi.mjs` gains
`FAKE_PI_HELP_HAS_MCP_CONFIG` (existing) and `FAKE_PI_HELP_HAS_NATIVE_MCP`, and
records the child env alongside argv.

## Risks / Trade-offs

- [`pi mcp` help substring changes shape in a future pi] → the regex is
  anchored (`/^\s*pi mcp(\s|$)/m`); a miss degrades to warning, never a crash.
- [A pi that supports `registerMcpServer` but hides `pi mcp` from help] →
  treated as `none` (warn + drop). Acceptable: the documented pi ≥ 0.99 path
  is authoritative; revisit only if observed.
- [Generated extension source drifts from pi's API] → it uses only
  `pi.registerMcpServer` and `node:fs`; the component test exercises the
  generated text, and the manual smoke covers the real binary.
- [A user's pi has both native MCP and the adapter installed] → adapter wins
  by D3, preserving today's working behavior.
- [Stale temp dirs after a pi-acp crash] → unchanged: dirs are inert, tmpdir
  cleanup reclaims them.

## Migration Plan

Additive within the carried patch: update `mcp-config.patch`, its tests, and
the package README; rebuild `nix build .#pi-acp`. Rollback is restoring the
previous patch file. No ptah crate or flake-input changes, so no lockfile or
hash churn; non-dev-shell users are unaffected until the package is rebuilt.

## Open Questions

None.
