# tsot — The Symbols of Teranos

A **1v1 collectible card game**, digital-first, where every card is identified by one of QNTX's canonical SEG symbols. The card on the back shows only the symbol; the face reveals everything else. Damage is mill. Costs are paid from your hand, deck, or graveyard. The game is designed to be answer-rich, tempo-driven, and amenable to mobile.

## Status

Pre-implementation. Rules, cards, and UX requirements are being authored in parallel with the Rust engine. The engine currently:

- Loads cards (`.lua` files) into typed Rust structs.
- Initializes a `GameState` with two players, per-player zones, 5-card opening hands.
- Detects deck-out loss (rule L.1).
- Computes effective stats with continuous modifier semantics (rule C.12).
- Moves cards between zones.

It does **not yet** play the game — no turn advancement, no actions, no combat resolution, no Lua-driven ability execution.

## Architecture

```
proto:  (none — schema is Rust + Lua tables)
engine: Rust crate (this repo root)         ← runs on native, WASM, and embeddable in mobile
cards:  Lua files in cards/                 ← each card is a Lua table; abilities will become Lua functions
rules:  RULES.md                            ← spec, atomic and reviewable
ux:     UX.md                               ← baseline interface requirements + engine API obligations
```

The Rust engine compiles to:

- Native binary (CLI for testing).
- `cdylib` (for WASM bindings — a future TS presentation layer will consume them).
- Mobile-compatible static library (planned: Tauri 2 or direct iOS/Android shells).

Cards are written in Lua because abilities are programs, not data. Each card is a self-contained `.lua` file returning a table with id, name, colors, type, cost, abilities, and stats. Abilities are currently strings; they will become functions when the engine grows event dispatch.

## Repo layout

```
tsot/
├── Cargo.toml         Rust crate manifest
├── Cargo.lock
├── flake.nix          Nix dev shell + package
├── RULES.md           game rules, atomic and ID'd
├── UX.md              interface baseline requirements
├── README.md
├── src/
│   ├── lib.rs         re-exports
│   ├── card.rs        Card type, Lua loader, enums
│   ├── game.rs        GameState, PlayerState, CardInstance, Phase, Zone, modifiers
│   └── main.rs        CLI smoke: load cards, init game, print state
├── cards/             29 cards as .lua files
└── frontend-garden/   archived v1 TS garden (single-player QNTX symbol tutorial)
```

## Building

```sh
cargo build               # native binary
cargo run                 # loads all cards, prints initial game state
cargo clippy --all-targets
cargo test                # (no tests yet)
```

Or via Nix:

```sh
nix develop               # dev shell with rustc, cargo, clippy, rust-analyzer, lua5.4
nix build                 # build the package
```

mlua bundles Lua 5.4 from source via the `vendored` feature; no system Lua install needed.

## Documents

- **`RULES.md`** — the rules of the game, organized by section (Format, Turns, Loss, Setup, Zones, Cards, Exclusions, Abilities, Responses, Control, Combat, Play, Visibility). Each rule has a stable ID (e.g., `U.6`, `B.7`).
- **`UX.md`** — baseline UX requirements (e.g., skip prompts when no response is possible) plus the engine API surface those requirements imply.
- **`cards/*.lua`** — card definitions. 29 cards in the current corpus.

## The archived v1 garden

`frontend-garden/` contains the original single-player QNTX symbol tutorial — a browser-based collection garden built with Bun, TypeScript, and `@qntx/glyphs`. The CCG direction superseded it; the garden is kept as an archive, not under active development.

To run the garden:

```sh
cd frontend-garden
bun install
bun run dev               # http://localhost:5180
```
