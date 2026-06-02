# tsot — The Symbols of Teranos

A **1v1 collectible card game**, digital-first, where every card is identified by one of QNTX's canonical SEG symbols. The card on the back shows only the symbol; the face reveals everything else. Damage is mill. Costs are paid from your hand, deck, or graveyard. The game is designed to be answer-rich, tempo-driven, and amenable to mobile.

## What's distinctive

- **Cards are programs, not data.** Each card is a self-contained `.lua` file. Abilities are real functions invoked through a sandboxed mlua VM.
- **Deterministic engine, journaled mutations.** Every state change is recorded; same seed → byte-identical game. Foundation for replay, save/load, AI search, and (eventually) multiplayer rollback netcode.
- **Choice as an oracle.** Cards ask questions through a trait. Sim uses a random oracle, tests use a scripted one. Same handler code, different decision sources.

## Status

Mid-engine. Plays a turn end-to-end including combat, fires Lua handlers, supports preview/rollback/replay/save-load. The simulator is driven by a Make-fronted CLI (`make help`):

- **`make evolve`** — one round of evolutionary deck search (~7-8 min wall, parallelized across cores) against a curated baseline gauntlet; saves top-5 evolved decks per round and writes an `evolve-report.html` showing card-presence per generation
- **`make report`** — aggregates all saved champions into `champions-report.html` (card frequency, clustering, fitness correlation, per-champion game-level sampling)
- **`make matchup-decks`** — round-robin grid over any directory of saved decks (default `baselines/`); HTML with win-rate matrix, turn distributions, event-firing breakdown, top-cards-by-play-frequency
- **`make curate-baselines`** — live re-evaluation of accumulated champions against the snapshot baselines; promotes winners
- **`make prune-champions`** — cluster champions by Jaccard, live-rank within each cluster, keep top-K per cluster, delete the rest; bounds gauntlet growth by (archetypes × K)
- **`make pool`** — static analytics dashboard of the card pool (color × cost × type × subtype × keyword distributions, plus per-card turn-played sparklines from a chained `tsot curve-sample` run) → `card-pool.html`
- **`make archetypes`** — Jaccard clustering of saved decks → `archetypes-report.html` (which decks form the same attractor)
- **`make probe [CARD_ID...]`** — side-by-side comparison of a card's declared variants via short pinned EAs; auto-discovers every card with a `variants` block if no id given → `balance-probe-report.html`. Long-form: `make probe-long`.
- **`make matchup-mcts`** — head-to-head between the existing Heuristic AI and a one-ply rollout MCTS that does journal-rollback search over Pattern B card-pick decisions. Defaults to asymmetric mode (two random baseline decks); `--handicap` forces MCTS onto the lower-fitness deck; `--deck PATH` runs a mirror match. MCTS wins ~76% in mirror, ~61% with a 0.025-fitness handicap.
- **`make evolve-mcts`** — like `make evolve` but the gauntlet opponent plays MCTS. Evolved decks have to beat strong play to score high. ~16× slower per fitness eval (~2-4h per round at default rollouts=5); tune via `EVOLVE_MCTS_ROLLOUTS=`.

**Engine state:** turn loop with combat, response windows + counterspells, statics (anthems / keyword grants / restrictions / cost reductions), full cost vocabulary (HAND / MILL / GRAVEYARD / SACRIFICE / ATTACHED + jewel/crystal/Clear-View substitutions), X-cost casts and activated abilities, card-variants schema with `make probe`, intent-aware AI targeting, **one-ply rollout MCTS as a second AI** driven by full-game journal rollback (every mutation site is journaled; `RigCreatureFreeHaste` was the last sim shortcut to gain its own journal variant). Detailed feature inventory and remaining gaps live in `LIMITATIONS.md`.

## Building & running

```sh
cargo build               # native binary
cargo build --release     # release build (used by the make targets)
cargo test
cargo clippy --all-targets

make help                 # list the simulator commands
make evolve               # one EA round; HTML report writes alongside
make report               # aggregate champion stats → champions-report.html
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
- **`STATIC.md`** — phased plan for continuous effects. Phase 1 + 2 + 3 + 3.5 done.
- **`JOURNAL.md`** — multi-session plan for mutation journaling, rollback, replay, save/load.
- **`EA.md`** — evolutionary deck search (the current primary simulation mode).
- **`src/sim/README.md`** — sim AI heuristics + game-runner internals.

## The archived v1 garden

`frontend-garden/` contains the original single-player QNTX symbol tutorial — a browser-based collection garden built with Bun, TypeScript, and `@qntx/glyphs`. The CCG direction superseded it; the garden is kept as an archive.

```sh
cd frontend-garden
bun install
bun run dev               # http://localhost:5180
```
