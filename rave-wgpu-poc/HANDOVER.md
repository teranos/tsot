# rave-wgpu-poc — handover

A proof-of-concept that renders a minimal slice of the rave world with
**wgpu 29 directly** — no Bevy, no winit — to answer one decision:

> Is it justified to abandon Bevy-rave and rebuild the renderer on wgpu?

This crate exists to turn that from an argument into a measurement. It
does **not** replace rave. It is a spike.

---

## The two questions it was built to answer

1. **Does a plain LDR wgpu scene render on the phone at real device
   resolution — the case Bevy-rave fails (black / OOM)?**
   Load `?` (default) on the actual device and read the `#status` bar.

2. **Can ONE wasm binary carry both WebGPU and WebGL2 with runtime
   selection?** Bevy could not — it gates WebGL2 shader defines at
   compile time (`not(feature="webgpu")`) and needs two bundles + a
   load-time shim. **wgpu can — and this crate proves it compiles.**

---

## What is VERIFIED (built + ran, here)

- **`webgpu` + `webgl` in one binary compiles.** `cargo build
  --target wasm32-unknown-unknown` is green with both wgpu features
  enabled at once (see `Cargo.toml`). This is the structural thing
  Bevy's compile-time gating made impossible. One binary, backend
  chosen at runtime via `RequestAdapterOptions` / the `?backend=`
  override in `src/lib.rs`.
- **The scene logic is correct and deterministic.** `cargo test`
  passes 2 tests: the Wang-hash forest is non-empty (>100 trees),
  deterministic across runs, and leaves the clearing free — the
  placement is ported 1:1 from `rave/src/trees.rs`.
- **The wgpu 29 render path type-checks end to end** — pipeline, depth
  buffer, instanced draws, per-frame camera uniform, RAF loop.
- **It builds to a loadable bundle: 2.3 MB wasm** (release,
  wasm-bindgen `--target web`, pre-`wasm-opt`). For scale, Bevy-rave's
  bundles run tens of MB.
- **It RUNS on a real WebGPU adapter — live, 48 fps.** Loaded headless
  (Chromium + SwiftShader WebGPU) it reports:
  `backend=BrowserWebGpu · 2560x1440 · trees=665 · draws=3 ·
  instances=1332 · ~48 fps`. The pipeline executes every frame — 665
  instanced trunks + 665 foliage + player + floor — with **no crash
  and no OOM**. This is the first direct evidence that a hand-rolled
  wgpu scene at this instance count runs where Bevy-rave black-screens.

## What is NOT yet verified (needs a real device)

- **Pixel correctness — that the forest actually looks right.**
  Headless Chromium does not composite a *WebGPU* canvas into a
  Playwright screenshot (the DOM `#status` overlay captures; the GPU
  canvas came back blank/white). The loop demonstrably runs (48 fps,
  3 draws, 1332 instances) but the pixels are unconfirmed until it is
  loaded on an actual device or a headed browser. **Do not claim it
  "renders correctly" until a real screenshot shows the trees.**
- **The forced-WebGL2 path.** `?backend=webgl2` panicked in headless
  with `CreateSurfaceError { inner: Hal(MissingDisplayHandle) }` at
  `src/lib.rs` surface creation. Strong suspicion this is the headless
  ANGLE/SwiftShader GL emulation (not a real browser's WebGL2), since
  `SurfaceTarget::Canvas` is the standard wgpu web-GL path — but it is
  UNCONFIRMED. First on-device task: load `?backend=webgl2` on the
  phone and see whether the fallback creates its surface. If it fails
  there too, the dual-backend claim needs a code fix (canvas → GL
  surface handling), not just a device.
- **Release wasm size after `wasm-opt`.** 2.3 MB is pre-opt;
  `wasm-opt` was absent in the build env, so `-Oz` was skipped.

---

## How to build & run

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.121   # MUST match the crate pin

cd rave-wgpu-poc
./web/build.sh                       # → dist/
python3 -m http.server -d dist 8080  # open on the phone, same LAN
```

- Default URL: wgpu tries WebGPU, falls back to WebGL2.
- `?backend=webgpu` / `?backend=webgl2` forces one backend from the
  same binary — the A/B rave needed two bundles for.
- The `#status` bar reports: `backend`, adapter name, drawable
  resolution, tree count, instance count, and FPS — so a single
  screenshot is the whole diagnostic, the same philosophy as rave's
  drawer.

---

## Architecture (≈600 lines total)

| file | role |
|------|------|
| `src/lib.rs` | wasm entry, GPU init, pipeline, RAF render loop, `#status` reporter. All wgpu lives here. |
| `src/scene.rs` | the ported world: `FLOOR_HALF`, `SPAWN_POS`, the Wang-hash forest sweep, the follow camera. **No wgpu** — pure logic, unit-tested. |
| `src/mesh.rs` | unit primitives (quad / cylinder / sphere) + `Vertex` / `Instance` / `CameraUniform` layouts. |
| `src/shader.wgsl` | one forward shader: directional + ambient, LDR out. Every construct is inside `downlevel_webgl2` limits, so the same shader runs on both backends. |
| `web/index.html` | sizes `#bevy` to `innerWidth*devicePixelRatio` (reproduces the 3840×2160@3 target that OOM'd Bevy), loads the module. |

**Deliberately LDR.** No HDR (`Rgba16Float`), no bloom mip chain, no
MSAA — exactly the render features whose memory was implicated in the
Bevy OOM. If this renders and Bevy doesn't, the delta is the cause.

---

## What is NOT ported (scope boundary — read before extending)

Present in rave, **absent here on purpose**:

- **Networking (bevy-libp2p / gossipsub / remote players).** This is
  the real cost of leaving Bevy — the laye plugins are Bevy-shaped.
  The POC renders a *static* world; it proves nothing about porting
  the net spine. That estimate is the next spike, not this one.
- **Input / movement.** `room.rs::move_player` (WASD + touch joystick)
  is not wired; the camera auto-orbits instead so a screenshot reads
  as 3D. The touch-drag math itself is trivial to port (it is already
  pure in `room.rs`).
- **The clearing furniture** (`floorplan.rs`: DJ booth, bar, truss,
  strobes), the campfire, trail, minimap, chat, drawer, audio,
  identity/IndexedDB. None are rendering-risk-bearing; they were left
  out to keep the spike honest and small.

---

## The decision this feeds

- **If it renders clean on the phone** → the black screen was Bevy's
  render stack (its render-target formats / pipeline-cache bloat), not
  the device. Abandoning Bevy's *renderer* becomes justified — and the
  single-binary dual-backend result means you also delete the
  two-bundle shim. Next spike: cost the bevy-libp2p → bare-stack port,
  because that, not the renderer, decides the total bill.
- **If it also blacks/OOMs at 3840×2160** → the problem was never
  Bevy; it is raw pixels on that device. The fix is a resolution /
  `scale_factor` cap (a 10-line change in rave), and a rewrite would
  inherit the same crash. Do not rewrite.

Either outcome is a real answer. That is what a spike is for.
