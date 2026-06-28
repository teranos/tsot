{
  description = "rave";

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
            sccache
            # bun bundles + type-checks the TypeScript modules under
            # rave/web/. Picked over node+esbuild because bun has zero
            # runtime install dance and sub-second cold builds for a
            # tree this size.
            bun
          ];

          RUSTC_WRAPPER = "sccache";
        };
      });
}
