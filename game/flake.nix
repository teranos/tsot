{
  description = "game.sbvh.nl — wgpu game. CDDA map corpus pinned as a build-time dependency, never vendored.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Nightly rust with rust-src (for -Z build-std) + the wasm target,
    # same shape as the repo-root flake.
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
        rust = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" ];
          targets = [ "wasm32-unknown-unknown" ];
        };

        # CDDA map corpus — a *dependency*, not vendored source. Pinned
        # to the stable letter release named in ./CDDA_RELEASE (e.g.
        # 0.I). Sparse: only the mapgen + palette subtrees, so any
        # building can be referenced without vendoring a single file.
        #
        # Provenance of record: owner/repo/rev + the content `hash`
        # below, which Nix verifies on every build. The first build
        # prints the real hash to paste in place of `lib.fakeHash`
        # (trust-on-first-use); bumping the release = edit CDDA_RELEASE,
        # blank the hash, rebuild, commit the new hash — all reviewable.
        cddaRelease = pkgs.lib.removeSuffix "\n" (builtins.readFile ./CDDA_RELEASE);
        # Exactly the files the game references — the same one manifest
        # build.rs and tools/fetch-cdda.sh read. Non-cone sparse so it's
        # file-level (~30 KB), not the whole 13 MB mapgen tree.
        cddaFiles = pkgs.lib.filter (s: s != "" && !pkgs.lib.hasPrefix "#" s)
          (pkgs.lib.splitString "\n" (builtins.readFile ./cdda-files.txt));
        cddaSrc = pkgs.fetchFromGitHub {
          owner = "CleverRaven";
          repo = "Cataclysm-DDA";
          rev = cddaRelease;
          sparseCheckout = cddaFiles;
          nonConeMode = true;
          # NAR hash of the sparse tree (the files in cdda-files.txt) at
          # rev 0.I, verified against the bytes CDDA ships at that
          # release. IMPORTANT: this hash tracks cdda-files.txt — adding
          # or removing a file changes the sparse set and this MUST be
          # recomputed, or `nix build .#cdda-src` fails and the deploy +
          # seer break. (Recompute: reproduce the listed files at 0.I,
          # `nix hash path --type sha256 --sri <dir>`.)
          hash = "sha256-kmTjhbwAiPaEbNdRG40Pgw67pbXczCFnWam7hve2Yl8=";
        };
      in {
        # The pinned corpus, exposed so CI / tooling can realise it
        # (`nix build .#cdda-src`) and point CDDA_SRC at the result.
        packages.cdda-src = cddaSrc;

        devShells.default = pkgs.mkShell {
          packages = [ rust pkgs.rust-analyzer pkgs.bun pkgs.git ];
          # build.rs reads this; no fetch script needed inside the shell.
          CDDA_SRC = cddaSrc;
          shellHook = ''
            echo "[game] CDDA corpus pinned to ${cddaRelease} (CDDA_SRC=$CDDA_SRC)"
          '';
        };
      });
}
