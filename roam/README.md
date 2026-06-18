# roam

See @CANONICAL.md for the canonical-world axiom (how world transformations are routed by identity).

What i want:

- Let's not break the 2nd law of thermodynamics.
- right now we have a cylinder world, and the dimensions also do not make sense. what i really want is the ability to dig through the entire planet and surface on the other side, somehow, i think it has to be possible to z-levels, but im not sure how it want to compromise yet. it seems like this is a very fundamental thing, so it matters in the sense that i want to get it right now, before adding a lot more complexity to worldgen.
- core mechanic, going into lab's finding mutations, find bionics, install them.
- Reproduction is important in this universe, as the human observer/player you get to continue with the next in line basically. the lifetime of a character in TSOT isn't terribly long. pass on traits, do embryo selection, go full designer baby, cross-species breeding, all of the good stuff. Some requirements apply like, access to flowers, perfume,
- each voxel is 0.5M by 0.5M (M = meters; so half a meter on a side) in my imagination, so a door would be 2 voxels wide and 4 high — that's 1M by 2M — making 8 voxels in total.
- You can chat, but it's proximate, so, think hybrid of habbo and eve online. you can also DM people you know directly. Social component of TSOT is crucial, could even say that it's going to be the main thing that will determine success of the game.
-
- WebAuthn, did,
- ActivityPub,
- ATProto,
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
- [~] v0.3 — swap BroadcastChannel for js-libp2p. Tabs across browsers see each other.
      Plumbing works (custom Bun.serve WS transport, libp2p mesh forms via relay, cross-tab gossip seen
      working for ~1s). Unstable — yamux aborts kill streams; see "Known issues / stability".
- [ ] v0.4 — TSOT cards spawn on ground; pick-up with Lamport conflict resolution; collection persists in IndexedDB. Introduces mlua + the emscripten quirks (SUPPORT_LONGJMP, build-std).
- [ ] v0.5 — match handshake: bump into another player → both run TSOT engine on their collected decks → result merged.
