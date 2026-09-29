# pi-acp (patched)

ACP adapter for the [pi](https://github.com/svkozak/pi-acp) coding agent,
carried as a flake output (`packages.<system>.pi-acp`) so the project
registry's `command = "pi-acp"` resolves from the dev shell's `PATH`.

## Why the patch

Upstream pi-acp accepts ACP `session/new { mcpServers }` but never wires
the servers into pi. ptah uses MCP servers to expose its typed-results
channel (`ptah_result_submit`), so without the wiring every `resultSchema`
script silently degrades: turns complete with `result = nil`.

`mcp-config.patch` (beside this file) closes the gap: it materializes the
stdio servers from `session/new` into a per-session `--mcp-config` file
(mode 0600, removed at session end) passed to pi. Servers pi can't take
(http/sse transports, or pi without the `--mcp-config` flag — provided by
the `pi-mcp-adapter` extension) are dropped with a warning, which is
ptah's documented degradation path.

Upstreaming is out of scope by decision, so the source is pinned to one
exact rev (`b0581c9`, v0.0.34) in `default.nix`.

## Bumping the pinned rev

Bumping the rev requires rebasing the patch by hand. The workflow:

1. Clone/refresh the upstream source at the new rev into the gitignored
   `.work/pi-acp` scratch dir.
2. Check out the pinned rev, apply `mcp-config.patch` (`git apply`),
   commit it, then `git rebase` onto the new rev; resolve conflicts and
   run the patch's own test suite there (`npm test`).
3. Export the updated patch with `git diff` over the touched files and
   replace `mcp-config.patch` with it.
4. Update `rev`/`hash`/`npmDepsHash` in `default.nix` (hashes from the
   failed build's error messages) and bump `version`.

Watch for new RPCs the rebased upstream issues during `session/new`: the
patch's `test/helpers/fake-pi.mjs` must answer them, or the component
tests fail. The 0.0.34 rebase added a `get_available_thinking_levels`
response for exactly this reason. 0.0.34 also requires pi v0.81.0+
(model-specific thinking levels).

Sanity-check the rebuild: `nix build .#pi-acp`, then a `resultSchema`
script against the `pi` agent from the dev shell.
