{
  flake-parts-lib,
  ...
}: {
  # Declare the shared option so other perSystem modules can use it.
  options.perSystem = flake-parts-lib.mkPerSystemOption ({
    lib,
    ...
  }: {
    options.ptahSrc = lib.mkOption {
      # The cleanSourceWith result (outPath + filter); no fitting public
      # lib type, so leave it unspecified.
      description = ''
        Cleaned repo source shared by every derivation that compiles the
        crate (release package, test checks, smoke check). Keeping ONE
        source matters: the workspace embeds non-Rust files at compile
        time (crates/ptah-check/src/defs.rs does
        include_str!("../../../.ptah/ptah.d.luau")), so a
        cargo-only source filter lets the dependency build and the test
        suite pass while the package build fails — exactly how `nix run`
        once regressed with `nix flake check` still green.
      '';
    };
  });

  config.perSystem = {pkgs, ...}: {
    ptahSrc = pkgs.lib.cleanSourceWith {
      src = ../.;
      filter = path: type:
        # Local runtime state and tooling configs — read from the
        # invocation dir at run time, never compile inputs — and nix/,
        # packaging only, so cargo builds stay insensitive to nix edits.
        # Exceptions inside .ptah/: the checked-in type definitions (a
        # genuine compile input via include_str! in src/cli.rs), the
        # workflow shims (test-covered code — tests/ptah_libs.rs runs
        # them against the mock agent, so they must survive in the
        # sandbox source; the directory itself must pass the filter or
        # the whole subtree is pruned), and the package manifest +
        # lockfile (the pin-guard test reads the lockfile to assert the
        # flake-pinned library matches the committed pin). The rest of
        # .ptah/ (generated packages, caches, local config) stays out,
        # so `nix run .` does not rebuild when local .ptah scripts or
        # config change.
        if pkgs.lib.hasSuffix "/.ptah" path
        then type == "directory"
        else if pkgs.lib.hasSuffix "/.ptah/ptah.d.luau" path
        then true
        else if pkgs.lib.hasSuffix "/.ptah/pesde.toml" path
        then true
        else if pkgs.lib.hasSuffix "/.ptah/pesde.lock" path
        then true
        else if pkgs.lib.hasSuffix "/.ptah/workflows" path
        then type == "directory"
        else if pkgs.lib.hasInfix "/.ptah/workflows/" path
        then type == "directory" || pkgs.lib.hasSuffix ".luau" path
        else if pkgs.lib.hasInfix "/.ptah/" path
        then false
        else
          !(pkgs.lib.elem (baseNameOf path) [
            ".git"
            "nix"
            "target"
            ".work"
            ".pi"
            "openspec"
            "result"
            ".direnv"
            "worktrees"
            ".agents"
            ".helix"
          ]);
    };
  };
}
