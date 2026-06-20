# tsot — The Symbols of Teranos

A **1v1 collectible card game**, digital-first. The card on the back shows only the symbol; the face reveals everything else. Damage is mill. Costs are paid from your hand, deck, or graveyard. The game is designed to be answer-rich, tempo-driven, and amenable to mobile.

Monorepo: [roam](roam/) (the game) + tsot (the autobattle engine). See [`roam/README.md`](roam/README.md) for the game, [`LIMITATIONS.md`](LIMITATIONS.md) for engine status, [`EA.md`](EA.md) for the evolutionary deck-search loop and its make targets.

## Play roam

```sh
cd roam
nix develop -c make wasm-serve
```

Opens at http://localhost:8080. See [`roam/CLAUDE.md`](roam/CLAUDE.md) for the architecture and `roam/`'s own `make help`.

## Engine development (tsot)

```sh
cargo build               # native binary
cargo build --release     # release build (used by the make targets)
cargo test
cargo clippy --all-targets

make help                 # list the simulator commands
```

Browser play:

```sh
# Needs emscripten on PATH (https://emscripten.org/docs/getting_started/downloads.html).
make wasm                 # release wasm bundle, stage into dist/
make wasm-serve           # build + serve dist/ on http://localhost:8080
make wasm-dev             # debug wasm with -g (preserves wasm names section)
make wasm-dev-serve       # build dev + serve
```

Via Nix:

```sh
nix develop               # dev shell
nix build                 # build the package
```
