
See @docs/CANONICAL.md for the canonical-world axiom (how world transformations are routed by identity).

See [Architecture.md](./Architecture.md) for the artifact axioms.

What i want:

In scope (1-hour universe):

- 1st play:
  - when a new player open's game.sbvh.nl, they are able to play immediately, as in the game just starts (no login screen), you can just move and walk
  - after you finish the 1st new-player-strip the 1st 9x1 strip, that's when you will be prompted to create an account in case you want to join the p2p game.
  - Ablity to set different color to yourself, or your name, or nick.
  - start with a ccg starter-deck in one of the colors.
  - you start with nothing
  - you progress into the time or era the active canonical world is in and weave into its existing through an alligned strip in order to join the crowd
- Strip:
  - **PARKED: strips is no longer load bearing.** Strips being new player on-ramp or minigame is fine; beyond that, it's an interesting concept for a separate project that would be a p2p racing game. World progression instead: time progresses for everyone at the same speed. (Bullets below kept as the original vision, superseded.)
  - a level or segment if you will is a long piece of terrain, like a strip of level, like 9 times longer than it is wide
  - the 9x1 strips are procedurally generated.
  - in the beginning series of strip representing underwater cellular life
  - more strips, at some point sandy beaches
  - a strip that represents the moment sea life becomes capabale of becoming terrestrial
  - spore-like progression
  - bronze era's strips, still 9x1
  - space age strips, we go into the universe
  - The last heath-death strips.
  - All players together in the big crunch until big bang, together deciding what crosses the seam (dna, tardigrade, etc).
  - reaching the end of the strip makes it so we enter into a transition into another strip
  - the way the universe progresses, is that you can stay on one strip for too long, if the strip ends, you go to the next one
  - so it supports both slow and fast play
  - in my imagination there was the assumption that everything and everyone is essentially always moving into one direction, if you catch my drift.
  - The system knows when to put players together or when to split strip. to split strip means to fork into two strips making a strip area not as busy.
- Demo flow:
  - find cards and flower like we do in roam.
  - I would like to be able to have an inventory like roam does.
  - the way things are setup today means that Flowers are a contested resource.
  - bump into players or npc's and start card game autobattle.
    - UCT — its the autbattler, that's what i want to see work in game (`ccg/src/sim/uct.rs`).
  - win cards, edit deck, see how well deck is performing.
- Time:
  - Clock/Watch/Time in-game.
  - you start without a watch.
  - ouroborous
- Movement:
  - On mobile, i would like to be able to move around in game.
  - descend a cliff and climb cliffs.
  - terrain — hills, cliffs, lakes, seas.
  - directional position (so flashlight makes sense).
- Combat:
  - I want a gun.
  - I want to shoot a gun.
  - gun targets automatically as we approach direction.
  - I want to be able to shoot the NPC, when its shot it should show the ! mark again (above the npc).
  - The enemies that we should be able to shoot down, those would have to be zombies.
- NPC:
  - NPC's following you.
  - NPC's running away from you (after you shoot in their direction or at them).
- Rendering:
  - for flowers i would like to see a nice 3d flower based on the roam algo (`roam/src/teranos/flower.rs`).
  - in terms of rendering, lights would be cool you know.
- Audio:
  - Walking closer to campfire should make us hear firewood crackle.
  - for audio, i wish we could control it from inside of the game, so no more python script (`game/tools/gen-rave-ogg.py`).
- Identity:
  - in laye we already have working login and persitent identity, this is important to have in game.
  - part of identity is being able to login with delta chat and bluesky as well.
  - part of identity is also things like ens.
  - a crypto wallet is part of identity.
- game should be multi-relay.
- You can chat, but it's proximate, so, think hybrid of habbo and eve online. you can also DM people you know directly.
- a core mechanic, going into lab's finding mutations, find bionics, install them.
- given that i want strip, i want somewhat better 3D.
- i imagine there to be a long road and elevation, and the player just going fast n the path until they reach some kind of destination.
  - shoes, boots, (running shoes, etc.)
    - I'm still missing the ability to go 'super fast'
- in my idea world you can open up game at any moment and be setup to race with a player and also play the ccg, this game is for multiple types of players.

FIX:

- when i bump into the npc the ! mark should be above the NPC, not above me, it's the NPC that is alerted, and its the player alerting it.
- campfire should not be a cube.
- the pin's are they pins? i dont see pin labels, and i dont see anything happen on mouse hover.

Open:

- Token? Token Sale ?
- WHat about a DAO ?
- What about the network state
- what about owning a piece of land or 9x1 ribbon
- What about mobieus strip?
- What about fractals?
- What about fractals in fractals?

Deferred:

- Reproduction is important in this universe, as the human observer/player you get to continue with the next in line basically. the lifetime of a character in TSOT isn't terribly long. pass on traits, do embryo selection, go full designer baby, cross-species breeding, all of the good stuff. Some requirements apply like, access to flowers, perfume,
- Social component of TSOT is crucial, could even say that it's going to be the main thing that will determine success of the game.
-
- Cross-device authority is UCAN, via `rs-ucan`. Capability delegation, not key transfer. Pairing pattern follows Fission ODD (PIN-confirmed handshake; the new device gets its own keypair + a delegated capability).
- Hardware-backed keys (WebAuthn / secure enclave) are deferred (M3 on the menu). Desirable for theft resistance; loses portability on browsers without WebAuthn.
- ATProto is the social / moderation layer, not the identifier (M2 on the menu). It binds an ATProto handle to a roam `did:key`. The moderation vision below is independent of the identifier choice — it layers on top.
  - for moderation, we allow for split realities to exist through different labelling,
  - meta-game: you may spend an hour in the labelling-service moderation soc and get rewarded for the audited work that occurred during this period.
  - Auditors audit labelling work.
  - Split realities are allowed to exist
  - i think split realities actually becomes way easier with the strips
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
  - afterlife hell and heaven cycles run in parralel to the existing world
  - we try to pull a "The good place" on people
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
- There should be mechanisms that make supporting the game thorugh posting about it on social media rewardable.
- Perception:
  - Flashlight.
  - hidden/visible rules, so you have roguelike visibility rules.
  - hidden area memory, hearing, footsteps indicator, sound indicators in hidden area.
