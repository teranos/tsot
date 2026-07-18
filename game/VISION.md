
The obelisk, the lake, hill, posters, wardrobe, jump, area to jump on, clickable lights, music, DJ booth with music, auditorium, conference hall, radio tower, spaceship, chemical waste dump site, zombie apocalypse, and a prophet that will save the universe.

the universe is cyclic. at the end of the universe everything gets a dithered datamoshed overlay effect — the loss of information — and only the information that crosses the seam carries into the big bang of the next universe. the next universe seam, if you will. this happens every hour. the minds of the players fuse when everything comes together, and only when no thought still wants to end it all do we accept the new together; the universe starts again, and the collective mind chooses where the asteroid goes — the asteroid that contains the basic elements of new life, which was seeded by the players. they encounter it near an active volcano on a habitable planet, like earth but not really earth.

See [The end of the universe](docs/the-end-of-the-universe.md).

In that world you begin as a single cell. That single cell becomes multiple cells, you evolve by adding onto or shedding your dna, you keep doing this until there's a real 3D world to explore. after which you play the TSOT CCG in the world by bumping into players to start a UCT autobattle match. you can win a card. cards are a contested resource and they can carry over outside of the universe — because the player is authenticated with multiple things like ActivityPub or ATProto. the ATProto moderation server / labelling service idea is still very important to the project.

roam should be deleted when game has everything roam has or if roam has nothing of vlue anymore in terms of ideas and more.

## Old Roadmap of Roam

- [x] v0.1 — local-only ✅ shipped. WASD square, walled map (50×40 tiles), camera follows + clamps, debug HUD.
  - game: WASD yes. Walled bounds no. HUD no.
- [x] v0.2 — two tabs see each other via BroadcastChannel (same browser). Proves the protocol round-trip.
  - game: no
- [~] v0.3 — cross-browser P2P. Players see each other across tabs and browsers.
  - [x] 0.3.1 — identity persists per browser, public relay dashboard.
    - game: 32-byte key persists in IndexedDB. did:key no. dashboard no.
  - [x] 0.3.2 — correctness pass: more than 2 players coexist, identity failures fail loud, dashboard stays fresh.
    - game: untested at >2. failure surfacing no.
  - [x] 0.3.3 — repo restructure: TSOT card-game content moves into `ccg/` so root holds shared axioms only (cuts the bleed surface that kept making roam architecture descriptions inherit TSOT specifics).
    - game: N/A
  - [x] 0.3.4 — identity slice: `did:key` as the user-facing identifier (PeerId derived from the same Ed25519 key); `roam::identity` module; `is_identified_self` / `is_identified_peer` predicate is the canonical-class runtime criterion. `research/IDENTITY.md` records the rs-ucan / Fission deep-read for the path ahead.
    - game: no
  - [x] 0.3.5 — M5: libp2p's verified gossipsub `source` surfaces to the application layer as a `did:key`; trust line moves from "what the payload says" to "what libp2p signed."
    - game: no (game doesn't own libp2p — this becomes a relaye/laye responsibility, but the guarantee game consumes is still absent)
  - [x] 0.3.6 — M6 + first routed transformation: flower pickup. Mutations route through `WorldClass::{Canonical, NonCanonical}`; identified players' pickups propagate via gossipsub and the tile stays empty for every other identified peer. End-to-end verified by `tests/m6_via_relayer.rs` (real relayer binary + native libp2p clients). M7 promotion deferred until guest-mode entry exists.
    - game: no. no flowers, no pickup, no canonical routing.
- [x] v0.4 — cards on the ground. Pick them up, collection persists. Depends on 0.3.6 — cards are world state; without canonical routing the axiom doesn't hold and non-canonical players grief canonical state.
  - game: no
- [x] v0.4.1 — eframe owns the canvas; right-click spawn menu + 16-font picker prove it.
  - game: Bevy owns the canvas; no spawn menu, no font picker
- [x] v0.4.2 — Minecraft-shape inventory in egui (hotbar + Tab-toggle extended grid), wall clock + build watermark moved into the egui surface, zoom (-/=/+) wired through Rust, render_gl re-asserts blend func per-frame against egui_glow's premultiplied override.
  - game: no
