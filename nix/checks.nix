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
        # git: the package-management fixtures clone their local git
        # index/repositories through gix, whose local transport shells
        # out to git-upload-pack.
        nativeBuildInputs = [pkgs.luau-lsp pkgs.git];
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

      nativeBuildInputs = [pkgs.luau-lsp pkgs.stylua];

      dontBuild = true;
      doCheck = true;

      checkPhase = ''
        runHook preCheck
        # StyLua defaults are the house style (no stylua.toml; the
        # nixpkgs pin is the version pin). .styluaignore keeps the
        # generated .ptah/ptah.d.luau out of this pass — its
        # byte-identity with `ptah types` output is a standing contract.
        stylua --check .
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

    # In-place `ptah check` gate over the repo's own entry scripts,
    # driven by the *release* package (shipped `ptah` with same-commit
    # embedded definitions, bundled mock-agent). The sandbox source
    # strips .ptah/config.toml by design, so the registry comes from a
    # synthesized HOME-level config defining the two agent names the
    # scripts reference (examples use `demo`, workflow shims `pi`);
    # discovery walks up from the unpacked source (finds nothing — the
    # store has no .ptah/) and falls through to it. The shims require
    # ../../../factory-components/**, so the whole library's literal
    # require graph is covered with no probe file. Zero-execution:
    # nothing spawns an agent; the registry entry is lint-resolution
    # only. luau-lsp rides along because `ptah check`'s typecheck pass
    # PATH-discovers it and exits 2 when absent, and the release
    # package ships none (same pattern as checks.ptah-tests).
    checks.ptah-check = pkgs.stdenv.mkDerivation {
      pname = "ptah-check";
      version = commonArgs.version;
      src = config.ptahSrc;

      nativeBuildInputs = [config.packages.ptah pkgs.luau-lsp];

      dontBuild = true;
      doCheck = true;

      checkPhase = ''
        runHook preCheck
        export HOME="$NIX_BUILD_TOP/ptah-check-home"
        export XDG_CONFIG_HOME="$HOME/.config"
        mkdir -p "$XDG_CONFIG_HOME/ptah"
        cat > "$XDG_CONFIG_HOME/ptah/config.toml" <<EOF
        [agents.demo]
        command = "${config.packages.ptah}/bin/mock-agent"
        args = []

        [agents.pi]
        command = "${config.packages.ptah}/bin/mock-agent"
        args = []

        # examples/ask.luau calls ptah.ask: the sandbox has no terminal,
        # so the check's interaction resolution needs an explicit
        # provider (this also exercises [ask] parsing in the release
        # binary).
        [ask]
        provider = "stdin"
        EOF
        for script in examples/*.luau examples/*/*.luau .ptah/workflows/*/main.luau; do
          echo "ptah check: $script"
          "${config.packages.ptah}/bin/ptah" check "$script"
        done
        runHook postCheck
      '';

      installPhase = ''
        mkdir -p $out
      '';
    };
  };
}
