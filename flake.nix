{
  description = "The Symbols of Teranos — 1v1 collectible card game engine";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "tsot";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # mlua's `vendored` feature builds Lua from C source — needs a C toolchain.
          # stdenv provides cc; no extra nativeBuildInputs required.

          meta = {
            description = "The Symbols of Teranos — 1v1 CCG engine (Rust + Lua cards)";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            lua5_4
          ];
        };
      });
}
