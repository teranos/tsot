# tsot — The Symbols of Teranos

Monorepo: [game](game/) (the game) + [ccg](ccg/) (the autobattle engine — TSOT). See [`game/README.md`](game/README.md) for the game, [`ccg/LIMITATIONS.md`](ccg/LIMITATIONS.md) for engine status, [`ccg/EA.md`](ccg/EA.md) for the evolutionary deck-search loop and its make targets.

## Play game

```sh
cd game
nix develop -c make wasm-serve
```

Opens at http://localhost:8080. See [`game/CLAUDE.md`](game/CLAUDE.md) for the architecture and `game/`'s own `make help`.

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
