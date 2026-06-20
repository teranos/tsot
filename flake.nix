{
  description = "The Symbols of Teranos — 1v1 collectible card game engine";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # rust-overlay gives us a `rust-bin` with configurable targets —
    # needed for the WASM_PLAN.md D6 wasm build which targets
    # wasm32-unknown-emscripten. nixpkgs' bare rustc ships without
    # cross-targets.
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
        # Nightly rust with the wasm target preinstalled. Nightly is
        # required for `-Z build-std=std,panic_abort` (set in
        # `.cargo/config.toml`'s `[unstable]` section) — we rebuild std
        # from source for the wasm target so its exception ABI matches
        # emscripten 5.x's `-fwasm-exceptions` default. Without it the
        # precompiled stable std's legacy `__cxa_find_matching_catch_*`
        # / `invoke_*` references don't link against new-ABI emcc.
        # `rust-src` is the source tree build-std consumes.
        rust = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" ];
          targets = [ "wasm32-unknown-emscripten" ];
        };
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "tsot";
          version = "0.1.0";
          # TSOT moved to ccg/ in 0.3.3. roam is a separate crate
          # (build with `nix develop` then `cd roam && cargo build`);
          # this package builds the TSOT engine only.
          src = ./ccg;

          cargoLock = {
            lockFile = ./ccg/Cargo.lock;
          };

          # mlua's `vendored` feature builds Lua from C source — needs a C toolchain.
          # stdenv provides cc; no extra nativeBuildInputs required.

          meta = {
            description = "The Symbols of Teranos — 1v1 CCG engine (Rust + Lua cards)";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # `rust` bundles cargo + rustc + rustfmt + clippy + the
            # wasm32-unknown-emscripten target (see `let rust = …`
            # at the top of this flake).
            rust
            rust-analyzer
            lua5_4
            # WASM_PLAN.md D6: `make wasm` / `make wasm-serve` need
            # emcc on PATH. Bundling here so the dev shell is the
            # single source of truth — no separate emsdk install.
            emscripten
            python3
            # `make assets` runs `elm make Main.elm --output=dist/bundle.js`.
            # elm-test runs unit tests under `assets/tests/` — required
            # by CLAUDE.md's "Strict TDD required" line; decoder + update
            # + view-shape tests for the Elm migration modules.
            elmPackages.elm
            elmPackages.elm-test
          ];

          # nixpkgs' emscripten ships with a read-only $NIX_STORE-side
          # cache. emcc wants a writable cache for ports + ports-build.
          # Point it at a project-local dir so the build doesn't fail
          # the first time it tries to materialize a Lua-side dependency.
          #
          # Also tell `cc-rs` to use emcc / em++ for any C/C++ compiled
          # for the wasm target. Without these, mlua-sys's build.rs
          # picks up the Nix-wrapped native clang (which injects
          # arm64-apple-darwin flags like -fzero-call-used-regs that
          # wasm clang rejects).
          shellHook = ''
            export EM_CACHE="$PWD/.em-cache"
            mkdir -p "$EM_CACHE"
            export CC_wasm32_unknown_emscripten=emcc
            export CXX_wasm32_unknown_emscripten=em++
            export AR_wasm32_unknown_emscripten=emar
            # cc-rs honors CRATE_CC_NO_DEFAULTS to skip the default
            # cflag set that the Nix wrapper picked up; otherwise its
            # native-toolchain flags still leak into the emcc invocation.
            export CRATE_CC_NO_DEFAULTS=1
            # mlua-sys vendors Lua's C runtime; Lua uses setjmp/longjmp
            # for error handling. emcc's default longjmp support emits
            # `__cxa_find_matching_catch_*` (LEGACY exception ABI) which
            # conflicts with the new wasm-exceptions ABI rustc forces at
            # link time. Routing setjmp/longjmp through wasm-native
            # exception instructions eliminates the legacy references:
            #   - -fwasm-exceptions enables the codegen path
            #   - -sSUPPORT_LONGJMP=wasm picks the wasm-native runtime
            # Setting via EMCC_CFLAGS (honored by emcc on every
            # invocation, regardless of caller) is more robust than
            # CFLAGS_wasm32_unknown_emscripten alone — cc-rs's env-var
            # lookup pathway has caching quirks in build scripts.
            # `-pthread` here is for atomic-instruction emission in the
            # C-side object files (Lua C runtime via mlua-sys). With
            # `-sSHARED_MEMORY=1` in the link args, wasm-ld refuses any
            # .o that wasn't compiled with the atomic/bulk-memory
            # features — `-pthread` is the umbrella that turns both on
            # at the C compiler level. No actual thread runtime spawns;
            # see PTHREAD_POOL_SIZE=0 in .cargo/config.toml.
            export EMCC_CFLAGS="-fwasm-exceptions -sSUPPORT_LONGJMP=wasm -pthread"
            export CFLAGS_wasm32_unknown_emscripten="-fwasm-exceptions -sSUPPORT_LONGJMP=wasm -pthread"
          '';
        };
      });
}
