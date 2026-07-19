# HANDOVER — branch `claude/lavapipe-graphics-8qk7zx`

For the next session reviewing this branch. Design, slices, and what
shipped are in [`game/docs/TERRAIN.md`](game/docs/TERRAIN.md) — read that
first. This note is only what isn't in there.

SC4-style terrain height for `game/`. All work is committed and pushed;
diff is `game/`-only; no PR opened yet. CI was last green at `28e9459`
(commits after it are the collision fix + docs).

## Open items a reviewer will question

- **Browser proof is the live site, not a captured frame** — headless
  Chromium capture stayed flaky (GPU init stalls under
  `--virtual-time-budget`). game.sbvh.nl was user-verified instead.
- **Collision is XZ-only for now; 3D is the target.** Real 3D physics
  (gravity, capsule collider, colliders with Y extent) is on the roadmap
  — a helicopter needs it. The XZ resolve on this branch is scaffolding
  to be lifted in a follow-on branch, not a locked-in choice. Static
  colliders still sit at authored `y`; ground-follow is a heightfield
  lookup, not integration. Both flip in the 3D-collision branch.
