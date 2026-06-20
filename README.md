# tsot — The Symbols of Teranos

A 1v1 collectible card game. Damage is mill. Costs are paid from your hand, deck, or graveyard.

Monorepo: [roam](roam/) (the game) + [ccg](ccg/) (the autobattle engine — TSOT). See [`roam/README.md`](roam/README.md) for the game, [`ccg/LIMITATIONS.md`](ccg/LIMITATIONS.md) for engine status, [`ccg/EA.md`](ccg/EA.md) for the evolutionary deck-search loop and its make targets.

## Play roam

```sh
cd roam
nix develop -c make wasm-serve
```

Opens at http://localhost:8080. See [`roam/CLAUDE.md`](roam/CLAUDE.md) for the architecture and `roam/`'s own `make help`.

## Engine development (tsot)

Enter the Nix dev shell first (provides rustc, emscripten, elm, caddy):

```sh
nix develop
```

Then:

```sh
cargo build               # native binary
cargo test
cargo clippy --all-targets

make help                 # list the simulator commands
```

The browser-based engine playtest tool (for testing tsot, not the player-facing game — that's roam):

```sh
make wasm-dev-serve       # debug wasm with names section + serve at http://localhost:8080
```

`make help` lists the other wasm variants and override flags.
