# HANDOVER — branch `claude/lavapipe-graphics-8qk7zx`

For the next session reviewing this branch. Design, slices, and what
shipped are in [`game/docs/TERRAIN.md`](game/docs/TERRAIN.md) — read that
first. This note is only what isn't in there.

SC4-style terrain height for `game/`. All work is committed and pushed;
diff is `game/`-only; no PR opened yet. CI was last green at `28e9459`
(commits after it are the collision fix + docs).

## Traps

- **Nightly required** — `cargo +nightly` everywhere (bevy_ecs 0.19 needs
  rustc ≥ 1.95). Plain `cargo` won't compile.
- **Headless render** (the only accepted proof) — `game-native` under
  lavapipe: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`,
  frames via `SEER_MULTI_FRAME_DIR` / `SEER_FRAMES`.
- **Before pushing, gate clippy on both targets** — the CI failure mode
  that keeps recurring:
  ```
  cargo +nightly clippy --lib -- -D warnings
  cargo +nightly clippy --lib --target wasm32-unknown-unknown -- -D warnings
  ```
- **No wasm-bindgen** — every browser call is an `env.*` import in
  `game/imports.allow`. This branch added none; reuse existing crossings.

## Open items a reviewer will question

- **Browser proof is the live site, not a captured frame** — headless
  Chromium capture stayed flaky (GPU init stalls under
  `--virtual-time-budget`). game.sbvh.nl was user-verified instead.
- **Collision is XZ-only** — deliberate: with real sim height, colliders
  at authored `y` never overlap a player on a hill.

House rules (repo CLAUDE.md): failing test first, errors surfaced never
swallowed, and the render PNG is the only accepted proof — accept the
user's testimony about a render without re-litigating.
