# pi-acp (ACP adapter for the pi coding agent), patched in-repo.
#
# Upstream pi-acp accepts ACP `session/new { mcpServers }` but never wires
# them through to pi, which silently degrades ptah's typed-results contract
# (`result = nil` for every resultSchema script). The carried patch
# (mcp-config.patch, beside this file) materializes stdio MCP servers into a
# per-session `--mcp-config` file for pi. Upstreaming is out of scope by
# decision, so the source is pinned to one exact rev: bumping the rev
# requires rebasing the patch — see README.md in this directory.
{
  perSystem = {pkgs, ...}: {
    packages.pi-acp = pkgs.buildNpmPackage {
      pname = "pi-acp";
      version = "0.0.34";

      src = pkgs.fetchFromGitHub {
        owner = "svkozak";
        repo = "pi-acp";
        rev = "b0581c9c1d675e634234674484247008b03d69b4";
        hash = "sha256-QRwxOtTZOY+Np3PkAoy2o2PrUzEqjItM/372sCPlSMo=";
      };

      patches = [./mcp-config.patch];

      nodejs = pkgs.nodejs_22;
      npmDepsHash = "sha256-BvLNtFfp1cMVjzWcMRSdhTqiJrTfbFoUbWkkPW9200o=";
      npmBuild = "npm run build";

      meta = {
        description = "ACP adapter for the pi coding agent (patched: ACP mcpServers wired through to pi)";
        homepage = "https://github.com/svkozak/pi-acp";
        license = pkgs.lib.licenses.mit;
        mainProgram = "pi-acp";
        platforms = pkgs.lib.platforms.unix;
      };
    };
  };
}
