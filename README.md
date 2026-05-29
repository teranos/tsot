# tsot — The Symbols of Teranos

A **1v1 collectible card game**, digital-first, where every card is identified by one of QNTX's canonical SEG symbols. The card on the back shows only the symbol; the face reveals everything else. Damage is mill. Costs are paid from your hand, deck, or graveyard. The game is designed to be answer-rich, tempo-driven, and amenable to mobile.

## Status

Mid-engine. The engine plays a turn end-to-end including combat. Cards are loaded from Lua but their abilities don't execute yet (no event/dispatch system). The bundled `cargo run` is a 1000-game simulator that exercises play + combat + deck-out across the corpus.

**What the engine does today:**

- Loads 32 cards (`.lua` files) into typed Rust structs (creature, instant, spell, artifact, environment).
- Initializes `GameState` per **F.2** (two players), **S.1** (5-card opening hand), **S.4** (50-card deck).
- Advances turns through the canonical phase order from **U.6** (Untap → Draw → Main1 → Combat → Main2 → End).
- Auto-actions per phase: untap (**U.2**), draw (**U.3**, **U.4**), damage clearing (**B.10**).
- Detects deck-out loss (**L.1**) on draw and on combat mill.
- Computes effective stats continuously per **C.12**.
- Moves cards between zones; tracks attached cards under each on-board instance.
- Plays creatures (HAND + MILL cost components, P.6 attachment, P.17 face-down).
- Resolves combat: declare attackers, declare blockers, damage exchange, deaths, B.2 mill on unblocked attacks.
- Checks 5 combat keywords: **flying**, **unblockable**, **haste**, **vigilance**, **defender**.
- Standard-format card filtering (test subtype excluded per **S.5**).

**What the engine does NOT do yet:**

- Execute Lua ability functions — all abilities are stored as strings; nothing fires triggered or activated abilities.
- Modifier dispatch — `Modifier::StatBoost` and `Modifier::GainsFlying` are recognized by effective-stat math, but no card adds them to instances.
- Other cost sources: `GRAVEYARD`, `SACRIFICE`, `SELF`.
- Other card-type plays: instant, spell, artifact, environment.
- Variable X cost (Hydra, Recast, etc.).
- Response windows (R.1) and the stack (R.2–R.7).
- Mulligan (S.2/S.3).
- Color/symbol/type mutations.
- Counter on the stack.
- P.8 attached-cards-go-to-exile on host death.

The `cargo run` output's "Pending mechanics" section enumerates every primitive currently zero in stats; each will become non-zero as its engine piece lands.

## Architecture

```
engine: Rust crate (this repo root)         ← runs on native, WASM, mobile
cards:  Lua files in cards/                 ← each card is a Lua table; abilities will become functions
rules:  RULES.md                            ← spec, atomic and reviewable
ux:     UX.md                               ← baseline interface requirements + engine API obligations
```

Cards are written in Lua because abilities are programs, not data. Each card is a self-contained `.lua` file returning a table with id, name, colors, type, subtypes, symbol, cost, abilities, and stats. Abilities are currently strings; they will become functions when the engine grows event dispatch.

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
│   ├── main.rs        sim: 1000 games, aggregate stats, last-game log
│   └── game/
│       ├── state.rs       types: GameState, Phase, Zone, CardInstance, Modifier, ...
│       ├── turn.rs        phase advancement: untap, draw, damage clear, turn cycle
│       ├── movement.rs    move_card (zone transitions)
│       ├── play.rs        play_card (creature, HAND + MILL costs, P.6 attachment)
│       ├── combat.rs      declare_attacker / declare_blocker / confirm_blocks / damage / deaths
│       └── test_helpers.rs  shared #[cfg(test)] fixtures
├── cards/             32 cards as .lua files
└── frontend-garden/   archived v1 TS garden (single-player QNTX symbol tutorial)
```

## Building & running

```sh
cargo build               # native binary
cargo run                 # 1000-game simulator with aggregate stats + last-game log
cargo run --release       # ~half the runtime
cargo clippy --all-targets
cargo test                # 49 tests across state / turn / movement / play / combat
```

Or via Nix:

```sh
nix develop               # dev shell with rustc, cargo, clippy, rust-analyzer, lua5.4
nix build                 # build the package
```

mlua bundles Lua 5.4 from source via the `vendored` feature; no system Lua install needed.

## Documents

- **`RULES.md`** — the rules of the game, organized by section (Format F, Setup S, Turns U, Loss L, Zones Z, Cards C, Exclusions X, Abilities A, Play P, Visibility V, Responses R, Control T, Combat B). Each rule has a stable ID (e.g., `U.6`, `B.7`).
- **`UX.md`** — baseline UX requirements (X.1–X.7) plus the engine API surface those requirements imply (X-E.1–X-E.5).
- **`cards/*.lua`** — card definitions. 32 cards in the current corpus.

## Combat keywords (B.11, B.14–B.17)

| Keyword | Effect |
|---|---|
| `flying` | Can only be blocked by other flyers or anti-flying cards |
| `unblockable` | Cannot be blocked |
| `haste` | Can attack the turn it enters BOARD (overrides B.3) |
| `vigilance` | Does not tap when attacking (overrides B.4) |
| `defender` | Cannot attack |

## The archived v1 garden

`frontend-garden/` contains the original single-player QNTX symbol tutorial — a browser-based collection garden built with Bun, TypeScript, and `@qntx/glyphs`. The CCG direction superseded it; the garden is kept as an archive, not under active development.

To run the garden:

```sh
cd frontend-garden
bun install
bun run dev               # http://localhost:5180
```
