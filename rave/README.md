# rave

Bevy + libp2p rave party. Walkable 3D room with a DJ booth, bar, toilets,
garderobe, dancefloor + truss + strobes. Peers see each other as
spheres via `rave-positions/v1` (10Hz gossipsub broadcast through
`relay.sbvh.nl`). Identity is an Ed25519 keypair in IndexedDB. Chat is
the open slice.

Deployed at https://rave.sbvh.nl/ via CI on push to `rave` or `master`
(paths filter `rave/**`). No local dev — Bevy compile cost lives in CI,
not on this machine. See `.github/workflows/deploy-rave.yml`.

## Module layout

| Path | Owns |
|------|------|
| `src/lib.rs` | App orchestrator + JS-bridge externs only. ~200 lines. |
| `src/room.rs` | Floor plane, `PlayerCell`, WASD/touch movement, follow camera. |
| `src/floorplan.rs` | DJ booth, speakers, bar, toilets, garderobe, walls, dancefloor, truss + 6 sweeping spotlights, 4 corner strobes. |
| `src/drawer.rs` | In-canvas diagnostic UI: FPS, error list, net stats, clock, build watermark. Toggle with `` ` `` or `\`. |
| `src/observability.rs` | Panic hook + tracing layer + typed-error pipeline. `ErrorLog` resource. |
| `src/net_glue.rs` | Bevy ↔ libp2p glue: `boot_net`, `publish_self_position`, `drain_net_events`, `RemotePlayers` + render. wasm32 only. |
| `src/net.rs` | libp2p Swarm wiring + wire types (`PeerId`, `Topic`, `NetError`, `NetEvent`, `RavePosition`). wasm32 only. |
| `src/error.rs` | Typed sacred-error helpers (`emit_region`, thread-local buffer, `err-rave-N` id namespace). |
| `src/identity.rs` | Ed25519 keypair load-from-bytes or generate-fresh, IndexedDB bridge externs. |
| `src/build_info.rs` | `COMMIT` + `BUILT_AT` consts populated at compile time. |
| `web/` | Bun + TypeScript modules: overlay, error decoder, IndexedDB bridge, screenshot, streaming wasm fetch. Bundles to `dist/main.<h>.js`. |
| `tests/` | (none in rave/; the native libp2p integration test lives in `crates/rave-positions-test/`). |
| `BEVY.md` | Bevy decisions + version-bump tracker. |

## Build

```
nix develop -c make wasm
```

Calls `cargo build --release --lib`, then `wasm-bindgen`, then
`bun build` on the TS layer, then content-hashes
`main.js` + `rave.js` + `rave_bg.wasm` and writes the final
`dist/index.html`.

Nix flakes only see git-tracked files. New files need `git add` before
`nix develop` sees them.
