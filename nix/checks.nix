{
  inputs,
  ...
}: {
  # Offline test suite: the integration tests drive the in-repo mock agent
  # only (no network). The source is the shared config.ptahSrc so the
  # examples and fixtures the tests run are the same tree the package is
  # built from.
  perSystem = {
    config,
    pkgs,
    ...
  }: let
    craneLib = (inputs.crane.mkLib pkgs).overrideToolchain config.rustToolchain;

    commonArgs = {
      pname = "ptah";
      version = (builtins.fromTOML (builtins.readFile ../crates/ptah-cli/Cargo.toml)).package.version;
      CARGO_BUILD_RUSTFLAGS = "-C debuginfo=0";
    };

    # Same derivation as in packages/ptah/default.nix (keep the arguments
    # byte-identical): dependencies build once and are shared between
    # the release package and the test suite.
    cargoArtifacts = craneLib.buildDepsOnly (commonArgs
      // {
        src = craneLib.cleanCargoSource ../.;
      });
  in {
    checks.ptah-tests = craneLib.cargoTest (commonArgs
      // {
        src = config.ptahSrc;
        inherit cargoArtifacts;
        # tests/analyze.rs runs the *real* luau-lsp through `ptah check`
        # (the embedded definitions under test) and discovers it via
        # PATH; PTAH_REQUIRE_REAL_LSP makes its absence a hard failure
        # here so the sandbox can never silently skip that contract.
        nativeBuildInputs = [pkgs.luau-lsp];
        env.PTAH_REQUIRE_REAL_LSP = "1";
      });

    # `nix flake check` evaluates packages but does not build them, so a
    # broken release build only surfaced on `nix build`/`nix run`. This
    # check closes that gap: it builds the actual flake package and
    # drives the same binary `nix run` would start — CLI entry points,
    # the compile-time-embedded type definitions, and one bundled
    # example round-tripped through the in-repo mock agent (mirrors
    # tests/examples.rs; fully offline).
    checks.ptah-smoke = pkgs.runCommand "ptah-smoke" {
      nativeBuildInputs = [config.packages.ptah];
    } ''
      set -e
      ptah --version
      ptah --help > /dev/null

      # The embedded definitions (include_str! of .ptah/ptah.d.luau in
      # src/cli.rs) must actually be in the release binary.
      ptah types | head -n1 | grep -q "type definitions"

      # End-to-end: run a bundled example against the mock agent with a
      # generated project registry, exactly like tests/examples.rs.
      work=$(mktemp -d)
      mkdir -p "$work/.ptah"
      cat > "$work/.ptah/config.toml" <<EOF
      [agents.demo]
      command = "${config.packages.ptah}/bin/mock-agent"
      args = []
      EOF
      (cd "$work" && ptah run "${config.ptahSrc}/examples/fanout.luau") > /dev/null

      touch $out
    '';

    # Static-analysis gate for the Luau surface: every bundled script
    # (examples, type-definition probe fixture, the Factory Components
    # library, and this repo's own workflow shims) must pass luau-lsp in
    # strict mode (per-file --!strict directives; no committed .luaurc)
    # against the repo definitions (.ptah/ptah.d.luau). Keeps examples
    # and the library honest in the same direction as the runtime probe
    # test.
    checks.ptah-analyze = pkgs.stdenv.mkDerivation {
      pname = "ptah-analyze";
      version = commonArgs.version;
      src = config.ptahSrc;

      nativeBuildInputs = [pkgs.luau-lsp];

      dontBuild = true;
      doCheck = true;

      checkPhase = ''
        runHook preCheck
        luau-lsp analyze --platform=standard \
          --definitions=.ptah/ptah.d.luau \
          examples/*.luau examples/*/*.luau crates/ptah-cli/tests/fixtures/*.luau \
          factory-components/std/*.luau \
          factory-components/components/*/*.luau \
          .ptah/workflows/*/*.luau
        runHook postCheck
      '';

      installPhase = ''
        mkdir -p $out
      '';
    };
  };
}
