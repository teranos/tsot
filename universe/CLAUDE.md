universe — the new home for the Bevy/ECS game. Fresh code, not a port of roam.

Current: cells-stage prototype. WASD + drag, eat algae to grow, water particles repel from the player, camera follows. Press `/` in-canvas for the diagnostic drawer (FPS, captured errors).

Workflow: edit, push to `bevy` branch, CI builds + deploys to https://universe.sbvh.nl/. No local dev — Bevy compile cost lives in CI, not on this machine.

Bevy version is pinned in `Cargo.toml`. One tracker line in `BEVY.md` for the next-minor bump trigger.

**Errors are sacred** — panics + Bevy WARN/ERROR tracing events surface in the in-canvas drawer via `LogPlugin.custom_layer`. LogPlugin's console output is preserved (wrapped, not replaced). No silencing.

**Observability first** — the drawer is the in-canvas equivalent of devtools. If you can't see it, you don't know about it. Press `/` to toggle.

Direction (deferred, see `roam/README.md` "what i want" for the long arc): multi-cell tide pool with peer cells over libp2p, germ-line identity persisting across cell deaths, Spore-style stage progression, ~60-min heat-death universe. The last 10 seconds of one universe are the first 10 of the next — Ouroboros. Endgame is retaining as much entropy as possible against the universal flatten, played through to the final 10 seconds.

Single-line git commits, no Claude attribution.
