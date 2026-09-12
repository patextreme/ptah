{
  inputs,
  ...
}: {
  perSystem = {
    config,
    pkgs,
    ...
  }: {
    devShells.default = pkgs.mkShell {
      packages = with pkgs; [
        config.rustToolchain
        luau-lsp
        luau
        stylua
        # Patched in-repo (nix/packages/pi-acp): the ACP adapter for the `pi`
        # agent in .ptah/config.toml. Keeping it in the shell means the
        # registry's `command = "pi-acp"` resolves via PATH with no config
        # edits.
        config.packages.pi-acp
      ];

      RUST_SRC_PATH = "${config.rustToolchain}/lib/rustlib/src/rust/library";
      RUST_BACKTRACE = 1;

      # The pinned Ptah Playbooks checkout the integration suite installs as
      # the `ptah_libs` package (tests/ptah_libs.rs) — the flake input, so
      # plain `cargo test` in the shell is fully offline.
      PTAH_LIBS_SRC = "${inputs.ptah-libs}";

      shellHook = ''
        echo "ptah devshell: $(rustc --version)"
      '';
    };
  };
}
