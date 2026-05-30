# tsot — The Symbols of Teranos

A **1v1 collectible card game**, digital-first, where every card is identified by one of QNTX's canonical SEG symbols. The card on the back shows only the symbol; the face reveals everything else. Damage is mill. Costs are paid from your hand, deck, or graveyard. The game is designed to be answer-rich, tempo-driven, and amenable to mobile.

## What's distinctive

- **Cards are programs, not data.** Each card is a self-contained `.lua` file. Abilities are real functions invoked through a sandboxed mlua VM.
- **Deterministic engine, journaled mutations.** Every state change is recorded; same seed → byte-identical game. Foundation for replay, save/load, AI search, and (eventually) multiplayer rollback netcode.
- **Choice as an oracle.** Cards ask questions through a trait. Sim uses a random oracle, tests use a scripted one. Same handler code, different decision sources.

## Status

Mid-engine. Plays a turn end-to-end including combat, fires Lua handlers, supports preview/rollback/replay/save-load. The simulator runs 3,600 games per `cargo run` (6 deck-variants × 6 matchups × 100 games; configurable via `TSOT_GAMES_PER_MATCHUP=<n>`; seeded via `TSOT_SEED=<n>`) and skips suicide plays via journal preview. An HTML report (`tsot-report.html`) writes per run with matchup heatmaps, per-variant breakdowns, and per-card win rates.

STACK and response windows are done (Phase 1 + 2): instants get cast in response, counterspell works, threat-aware AI policy fires combat tricks during opponent's combat. STATIC Phase 1 (continuous stat anthems) is done — battle-captain and goblin-warlord modify stats on the board. Spell (Instant + Sorcery via timing) routes through `play_card`. Artifact lands in pool as pitch fuel. **Remaining gaps**: activated abilities (`T: ...`), STATIC Phase 2-4 (keyword grants, restrictions, replacements), delayed triggers, a targeting layer, SACRIFICE/SELF cost sources, Environment type. See `LIMITATIONS.md` for the tight current list.

## Building & running

```sh
cargo build               # native binary
cargo run                 # 1000-game simulator with stats + last-game log
cargo run --release       # ~half the runtime
cargo test
cargo clippy --all-targets

TSOT_SEED=42 cargo run    # reproducible run
TSOT_REPLAY_OUT=last.json cargo run    # dump last game's replay
```

Via Nix:

```sh
nix develop               # dev shell
nix build                 # build the package
```

mlua bundles Lua 5.4 from source via the `vendored` feature; no system Lua install needed.

## Documents

- **`RULES.md`** — the rules of the game. Each rule has a stable ID (e.g., `U.6`, `B.7`).
- **`UX.md`** — baseline UX requirements and the engine API surface those imply.
- **`LIMITATIONS.md`** — what the engine can't do today.
- **`LUA.md`** — phased plan for card-ability execution.
- **`STACK.md`** — phased plan for response windows. Phase 1 + 2 done.
- **`STATIC.md`** — phased plan for continuous effects. Phase 1 done.
- **`JOURNAL.md`** — multi-session plan for mutation journaling, rollback, replay, save/load.

## The archived v1 garden

`frontend-garden/` contains the original single-player QNTX symbol tutorial — a browser-based collection garden built with Bun, TypeScript, and `@qntx/glyphs`. The CCG direction superseded it; the garden is kept as an archive.

```sh
cd frontend-garden
bun install
bun run dev               # http://localhost:5180
```
