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
        # Fetch the whole mapgen + palette SUBTREES — NOT the exact files
        # in cdda-files.txt. This decouples the fetch from the manifest:
        # adding a building is one line in cdda-files.txt and does NOT
        # change what's fetched, so this hash only ever changes on a
        # CDDA_RELEASE bump — never on a corpus edit. (An earlier version
        # sparse-fetched exactly cdda-files.txt, which coupled the hash to
        # the file list and silently broke deploy + seer whenever a
        # building was added.) build.rs picks the exact files to embed.
        # Keep these patterns identical to tools/fetch-cdda.sh.
        cddaSrc = pkgs.fetchFromGitHub {
          owner = "CleverRaven";
          repo = "Cataclysm-DDA";
          rev = cddaRelease;
          sparseCheckout = [ "data/json/mapgen" "data/json/mapgen_palettes" ];
          nonConeMode = true;
          # NAR hash of those two subtrees at CDDA_RELEASE. Stable across
          # corpus edits; recompute only when CDDA_RELEASE changes.
          hash = "sha256-DXXfU9uFwW1BrWkvfardBC3KAIM2rOj7T4QgKGRFBv8=";
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
