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

**Done in the engine:**
- **STACK Phase 1 + 2** — response windows, counterspell, threat-aware AI combat tricks.
- **STATIC Phase 1 + 2 + 3 + 3.5** — stat anthems, keyword grants (flying, haste, vigilance), state-reading predicates, source-only / attached-host scopes, kind / has-keyword filters, action restrictions (cannot-attack, cannot-be-cost-paid), cost-reduction modifiers.
- **Costs** — HAND, MILL, GRAVEYARD, SACRIFICE (with kind filter); P.24a jewel tap and P.24b crystal tap as HAND-payment substitutions.
- **Card types routed by play_card** — Creature, Spell (Instant + Sorcery via timing), Artifact (with the no-summoning-sickness P.25 rule).
- **Sim AI heuristics** — Pattern B multi-noncreature per turn, play-priority scoring (cost-reducers + anthems land first), smart-pitch, smart-discard, smart-target, trade-up block policy, investment-aware sacrifice picker.

**Remaining gaps** (see `LIMITATIONS.md`): activated abilities (`T: ...`), targeting layer, phase-entry / delayed triggers, SELF cost source, Environment type, STATIC Phase 4 (replacement effects), OnDealtDamageToPlayer event, static-recompute on attached-set change.

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

## The archived v1 garden

`frontend-garden/` contains the original single-player QNTX symbol tutorial — a browser-based collection garden built with Bun, TypeScript, and `@qntx/glyphs`. The CCG direction superseded it; the garden is kept as an archive.

```sh
cd frontend-garden
bun install
bun run dev               # http://localhost:5180
```
