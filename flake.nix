{
  description = "ptah — Luau-scripted multi-agent orchestration over the Agent Client Protocol";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane = {
      url = "github:ipetkov/crane";
    };

    flake-parts = {
      url = "github:hercules-ci/flake-parts";
    };

    # The shared workflow library (Ptah Playbooks), pinned to the commit the
    # committed `.ptah/pesde.lock` resolves. The dev shell and the nix gates
    # install it offline from this checkout; a `ptah package update` must be
    # paired with `nix flake update ptah-libs` — the pin-guard test in
    # tests/ptah_libs.rs asserts the two pins agree.
    ptah-libs = {
      url = "github:patextreme/ptah-libs/0a282f942af9da6c8b1387c2aebfff3687ee4c11";
      flake = false;
    };
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      imports = [
        ./nix/toolchain.nix
        ./nix/source.nix
        ./nix/packages/ptah
        ./nix/packages/pi-acp
        ./nix/devshell.nix
        ./nix/apps.nix
        ./nix/checks.nix
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
    };
}
