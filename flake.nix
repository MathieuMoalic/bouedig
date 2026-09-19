{
  description = "Bouedig - full-stack Dioxus app (Axum backend, Web + Android clients)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , rust-overlay
    , flake-utils
    ,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # One toolchain for every build target of the workspace:
        #  - host (Linux x86): backend server
        #  - wasm32: web client
        #  - aarch64/armv7/x86_64/i686: Android client
        toolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" ];
          targets = [
            "wasm32-unknown-unknown"
            "aarch64-linux-android"
            "armv7-linux-androideabi"
            "i686-linux-android"
            "x86_64-linux-android"
          ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            toolchain
            pkgs.just
            pkgs.dioxus-cli
            pkgs.cargo-binstall
            pkgs.sqlite
            pkgs.geckodriver
            pkgs.firefox
            pkgs.android-tools
            pkgs.binaryen # wasm-opt, required by `dx build --release`
          ];

          # Let geckodriver locate the Nix firefox and write to its profile dir.
          env = { };

          shellHook = ''
            export MOZ_HEADLESS=1
            export DIOXUS_ACTIVE_OVERLAY="nix"
          '';
        };
      }
    );
}
