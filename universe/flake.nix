{
  description = "bevy-canvas-spike — Bevy 0.18 canvas-attach proof, sealed from roam";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-unknown-unknown" ];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rust
            rust-analyzer
            wasm-bindgen-cli
            trunk
            sccache
          ];

          # sccache caches compilation across cargo clean. Without it,
          # every cleanup re-pays the cold Bevy compile.
          RUSTC_WRAPPER = "sccache";
        };
      });
}
