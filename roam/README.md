# roam

<!-- IDENTITY MENU (roam/docs/IDENTITY.md):
       C4 — write a player-facing one-pager "what identity means here." UX, not spec.
       D5 — one-paragraph CHANGELOG-style "the why" for the identity choice. -->

See @docs/CANONICAL.md for the canonical-world axiom (how world transformations are routed by identity).

What i want:

- Let's not break the 2nd law of thermodynamics.
- right now we have a cylinder world, and the dimensions also do not make sense. what i really want is the ability to dig through the entire planet and surface on the other side, somehow, i think it has to be possible to z-levels, but im not sure how it want to compromise yet. it seems like this is a very fundamental thing, so it matters in the sense that i want to get it right now, before adding a lot more complexity to worldgen.
- core mechanic, going into lab's finding mutations, find bionics, install them.
- Reproduction is important in this universe, as the human observer/player you get to continue with the next in line basically. the lifetime of a character in TSOT isn't terribly long. pass on traits, do embryo selection, go full designer baby, cross-species breeding, all of the good stuff. Some requirements apply like, access to flowers, perfume,
- each voxel is 0.5M by 0.5M (M = meters; so half a meter on a side) in my imagination, so a door would be 2 voxels wide and 4 high — that's 1M by 2M — making 8 voxels in total.
- You can chat, but it's proximate, so, think hybrid of habbo and eve online. you can also DM people you know directly. Social component of TSOT is crucial, could even say that it's going to be the main thing that will determine success of the game.
-
- Identity is `did:key` (Ed25519 public key, multibase-encoded). The libp2p `PeerId` is the same key in a different encoding — derived, not separate. Persistence is the 32-byte private key in IndexedDB. See `docs/IDENTITY.md` for the menu, `research/IDENTITY.md` for the why.
- Cross-device authority is UCAN, via `rs-ucan`. Capability delegation, not key transfer. Pairing pattern follows Fission ODD (PIN-confirmed handshake; the new device gets its own keypair + a delegated capability).
- Hardware-backed keys (WebAuthn / secure enclave) are deferred (M3 on the menu). Desirable for theft resistance; loses portability on browsers without WebAuthn.
- ATProto is the social / moderation layer, not the identifier (M2 on the menu). It binds an ATProto handle to a roam `did:key`. The moderation vision below is independent of the identifier choice — it layers on top.
  - for moderation, we allow for split realities to exist through different labelling,
  - meta-game: you may spend an hour in the labelling-service moderation soc and get rewarded for the audited work that occurred during this period.
  - Auditors audit labelling work.
  - Split realities are allowed to exist
  - The Source code of the game will contain defaults that are for now governed by me, at some point there will be a more formalised process.
  - People can fork the source code any time and set their own defaults.
  - Players can set their own preferences in the game.
- Salt sources (and salt starved)
- Items defined as Lua:
  - food preservation (using salt), otherwise food would rot rather fast.
  - fishing, piers, harbors, boats
  - bread, wheat, beer, bakery, brewer
  - grapes, wine
  - glassware, cutlery
  - Apples, cider
  - wood barrels, glass- copper- iron- clay jars
  - flowers, perfume, beeswax, candles, bees, honey,
  - Cheese, milk, cows, goats, husbandry
  - Textiles
    - materials
      - silk,   hemp, cotton, wool, leather, rubber
    - Level of craftsmanship
    - socks
    - pants
    - underwear
    - shirts
    - jackets
    - shoes, boots, (running shoes, etc.)
      - I'm still missing the ability to go 'super fast'
    - gloves
    - backpacks
  - Spices
  - Sugar
- Becoming King
- Becoming Tyrant
- Afterlife
  - Going to hell
  - Going to Heaven
  - Neither Heaven or Hell, possess someone.
- Protest and Revolution
- Propaganda, NPC's are easy to influence, Doing so it part of the game. Use pieces of media in order to inhabit the minds of your subjects. Not all Propaganda is lies.
- Zombies and where they come from?
  - Human Zombies, Dog, cat zombies, Goblin Zombies, elf Zombies, etc.
- I guess in teranos, its not like there is an incredible amount of zombies, i guess a zombie bite would infect you, it takes a while to turn into a zombie, this world has a lot of answers to zombie infections actually. The world understands that its a type of virus, and that it's deployed politically by certain groups.
- There is some special material in this world, (microbots?), and it requires very little energy as well
  - probably something from an older ancient civ,
  - somehow it seems to be able to manufacture a wide variety of things,
  - and is reconfigurable
- Solar power is abundant,
  - batteries help conserve excess power.
  - various levels of batteries
- Electric wiring (copper probably)
- lights, automate doors, conveyor belts
- Gears, bolts,
- Oil, crude, light, kerosene,
- representative Perspective,
  - Personal, Household, Neighborhood, Village, City, Region, State
  -

## Roadmap

- [x] v0.1 — local-only ✅ shipped. WASD square, walled map (50×40 tiles), camera follows + clamps, debug HUD.
- [x] v0.2 — two tabs see each other via BroadcastChannel (same browser). Proves the protocol round-trip.
- [~] v0.3 — cross-browser P2P. Players see each other across tabs and browsers.
  - [x] 0.3.1 — identity persists per browser, public relay dashboard.
  - [x] 0.3.2 — correctness pass: more than 2 players coexist, identity failures fail loud, dashboard stays fresh.
  - [x] 0.3.3 — repo restructure: TSOT card-game content moves into `ccg/` so root holds shared axioms only (cuts the bleed surface that kept making roam architecture descriptions inherit TSOT specifics).
  - [x] 0.3.4 — identity slice: `did:key` as the user-facing identifier (PeerId derived from the same Ed25519 key); `roam::identity` module; `is_identified_self` / `is_identified_peer` predicate is the canonical-class runtime criterion. `research/IDENTITY.md` records the rs-ucan / Fission deep-read for the path ahead.
  - [x] 0.3.5 — M5: libp2p's verified gossipsub `source` surfaces to the application layer as a `did:key`; trust line moves from "what the payload says" to "what libp2p signed."
  - [x] 0.3.6 — M6 + first routed transformation: flower pickup. Mutations route through `WorldClass::{Canonical, NonCanonical}`; identified players' pickups propagate via gossipsub and the tile stays empty for every other identified peer. End-to-end verified by `tests/m6_via_relayer.rs` (real relayer binary + native libp2p clients). M7 promotion deferred until guest-mode entry exists.
- [x] v0.4 — cards on the ground. Pick them up, collection persists. Depends on 0.3.6 — cards are world state; without canonical routing the axiom doesn't hold and non-canonical players grief canonical state.
- [x] v0.4.1 — eframe owns the canvas; right-click spawn menu + 16-font picker prove it.
- [x] v0.4.2 — Minecraft-shape inventory in egui (hotbar + Tab-toggle extended grid), wall clock + build watermark moved into the egui surface, zoom (-/=/+) wired through Rust, render_gl re-asserts blend func per-frame against egui_glow's premultiplied override.
- [~] v0.5 — Bevy + 3D voxel render. The world model is already 3D-shaped (`tile_at(x, y, z)`, `surface_z`, the voxel framing in "What I want" above); the render catches up. Bevy owns the frame loop + wgpu pipeline + camera + mesh + lighting; `bevy_ecs` lights up the object-identity axiom (entities + Location components — pickup is `remove::<OnGround>().insert::<InInventory>()` on the same entity). **egui drops out entirely**: the existing `ui/mod.rs` rewrites straight into Bevy's in-core `bevy_ui` (Flexbox-based, ECS-native, no adapter). Worldgen + net + identity unchanged. Bevy pinned at 0.18 (not 0.19) so `bevy_voxel_world` is reachable at v0.5.5 without waiting. See `docs/adr/0003-bevy.md`. Phased:
  - [x] 0.5.0 — wasm spike: minimal Bevy `App::new()` compiles alongside libp2p + eframe in the roam wasm bundle. Verified 2026-06-22. Required loosening `sacred-error` + `tsot-card` serde pins from `=1.0.219` to `1.0.219` (Bevy's `hashbrown 0.16` transitive wants ≥1.0.220).
  - [ ] 0.5.1 — port `ui/mod.rs` to `bevy_ui` (not `bevy_egui`). eframe + egui + egui_glow all leave Cargo.toml. Hotbar / spawn menu / font picker / clock / build watermark all rewrite as `bevy_ui` entity trees. `sickle_ui` (declarative builder on top of `bevy_ui`) considered if the verbosity bites.
  - [ ] 0.5.2 — port `render_gl` to Bevy. World stays top-down 2D for this slice; render rewrites as Bevy materials + meshes so the visual baseline is preserved before the dimensionality shift.
  - [ ] 0.5.3 — `World` migrates to ECS. Player, peers, flowers, cards become entities + components. The object-identity axiom in `docs/UI.md` falls out of one query + one system.
  - [ ] 0.5.4 — click-to-pickup + drag + pickup animation. Falls out of ECS + Bevy input + position interpolation + Bevy's in-core `bevy_picking`. The hotbar+world hit-test seam closes.
  - [ ] 0.5.5 — 3D camera + voxel rendering. `bevy_voxel_world` if 0.18-ready (currently on main), otherwise `block-mesh-rs` + hand-rolled chunking. Lighting + depth cue. Digging through the planet becomes mechanically possible.
- [ ] v0.6 — bump into a player, autobattle. roam suspends, hands deck states to ccg, plays out, returns the result. The two wasm bundles coexist in one tab via `crates/tsot-card/` (shared `CardId`).
- [ ] v0.7 — M8 cross-device delegation via `rs-ucan`. PIN-confirmed pairing (Fission ODD), new device gets its own keypair + a delegated capability. Natural demo: "second device plays as you in canonical Teranos."
