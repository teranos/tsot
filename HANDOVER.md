# HANDOVER — laye-p2p-integration

Branch: `laye-p2p-integration`, 7 commits ahead of `origin/master @ 8a72f34`.

## What this branch is for

Wire laye-p2p into game so the browser tab runs libp2p in a
sibling wasm module (from laye) and game.wasm's env.* boundary
consumes bytes through it. WebSocket-to-relaye stays as an
automatic fallback only if laye fails to load or init.

Carry-along work on the same branch:
- Multi-branch deploys for game.sbvh.nl (branch pushes → `/<branch>/`, master → root).
- In-game version picker (press 9).
- Bevy-owned sound toggle button in the UI overlay pipeline.
- Repo-wide URL-flag tear-out.

## Commits (oldest → newest)

- `c10b2f7` — initial laye-p2p path behind `?p2p=laye`. Superseded by 406f8db (default-on) and 2d8b294 (same-origin).
- `c80fda6` — deploy-game.yml adopts laye's branch-prefix pattern. master → root (no --delete), other branches → `s3://.../<branch>/` with `--delete` scoped to the prefix. versions.json regenerated from `aws s3api list-objects-v2 --delimiter '/'`. In-game version picker (fetches `/versions.json`, DOM-built list, current branch highlighted cyan).
- `b28e53b` — picker link points at `/<branch>/index.html` (CloudFront serves default-root-object only at the true root, not per-subdir). versions.json's jq filters `"assets"` out.
- `406f8db` — Bevy sound button in the UI overlay pipeline (top-right corner, rising-edge tap toggles `audio::AUDIO_MUTED`). Every audio play path (`play_thump/pock/bunk/crackle`, `play` for music) gates on `is_muted()`. `music_gate_system` calls `audio::stop`/`play` on transition so the running BufferSourceNode goes silent. render_web bumps `ui_instance_buf` to 9 × `DpadInstance`. URL-flag tear-out: `?p2p`, `?sound`, `?proxy`, `?nodebug` gone. `m` keydown gone.
- `2d8b294` — laye artifacts fetched from `https://laye.sbvh.nl/` into game's dist during CI. index.html imports `./laye_p2p.js` same-origin. Cross-origin wasm-bindgen streaming instantiate was silently failing → `laye.init()` threw before installing DOM overlays → chat window never appeared. Also restores the axiom sections in `game/CLAUDE.md` ("No URL flags, no hidden features" + "In-game UI is Bevy — not HTML, not JS") that were stripped by mistake.
- `1c58346` — clarify comment: same-origin isn't "the reliable route", it's the only route.

## Deploy pipeline (as of this branch)

- Push to master → `s3://game-sbvh-static/` root. No `--delete`.
- Push to any other branch → `s3://game-sbvh-static/<branch>/`. `--delete` scoped to the branch prefix.
- `versions.json` regenerated after every push. Deny-list filters `"assets"`.
- CloudFront invalidation scoped to `/<prefix>*` + `/versions.json`.
- Laye artifacts (`laye_p2p.js`, `laye_p2p_bg.wasm`) curl'd from `https://laye.sbvh.nl/` into `game/web/dist/` after `bun run build.ts`. Served same-origin.

## Current live state on game.sbvh.nl

Empty of this branch's deploy. A `stamp-template` branch pushed
twice (2026-07-13 18:07 and 19:33 UTC) with **master's old
workflow** (no branch-prefix), which `aws s3 sync --delete` to
root and wiped `/laye-p2p-integration/*` + `/versions.json`.

- `https://game.sbvh.nl/` → 200 (stamp-template's build; probably cross-origin laye, likely broken chat/p2p)
- `https://game.sbvh.nl/laye-p2p-integration/index.html` → 403
- `https://game.sbvh.nl/versions.json` → 403

## Recurring risk

Master's workflow lacks branch-prefix logic. Any push to any
branch that inherited master before `c80fda6` will
`aws s3 sync --delete` to root and wipe subdirs. This branch's own
pushes are safe (its workflow is the new one) but any sibling
branch push after this one wipes it again.

**Durable fix: land the `deploy-game.yml` change from `c80fda6` on
master.** Cherry-pick, or merge this whole branch.

## How to test (once redeployed)

1. Re-push the branch to trigger CI: `git commit --allow-empty -m "re-deploy" && git push`.
2. Open two tabs on `https://game.sbvh.nl/laye-p2p-integration/index.html`.
3. Expect: two blue dots moving in each other's views (p2p via laye); laye chat overlay bottom-right; pressing `9` opens the version picker top-right.
4. If chat doesn't appear or peers don't see each other → browser console for laye init errors.

## Open / not done

- **Jukebox** — the sound-toggle button was going to be replaced by a world-space jukebox entity that toggles music on click. Not started. Sound button is still the UI affordance.
- **viewer/src/App.svelte's `?sha=`** — deep-link content addressing (not a feature flag). Left alone during the URL-flag audit; call whether it needs to go too.
- **Merge to master** — this branch or its workflow change should merge to master to stop the deploy-race with sibling branches.

## Files changed on this branch (vs origin/master)

- `.github/workflows/deploy-game.yml`
- `game/CLAUDE.md`
- `game/src/audio.rs`
- `game/src/campfire.rs`
- `game/src/dpad.rs`
- `game/src/lib.rs`
- `game/src/render_web.rs`
- `game/src/sound_button.rs` (new)
- `game/web/index.html`
- `game/web/style.css`
- `game/web/src/main.ts`
- `ccg/frontend-garden/src/debug.ts` (`?nodebug` removed)
</content>
</invoke>