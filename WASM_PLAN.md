# tsot — wasm + multiplayer + mobile plan

> Update by crossing through (`~~task line~~`) whenever you finish a task.
> Task descriptions ≤ 3 lines each. Critical path: D1 → D4 → D5.

## Phase D — wasm browser game

- [x] ~~**D1: GameSession global in wasm_ffi.**~~
  ~~`RefCell<Option<GameSession>>` thread_local holding CardRegistry + GameState~~
  ~~+ Arc<HumanInterface> + (prompt_rx, action_tx). Single-tab single-game.~~

- [x] ~~**D2: tsot_start_game FFI.**~~
  ~~Parse JSON args (seed, deck_a_ids[], deck_b_ids[], opp_ai), build registry +~~
  ~~decks + GameState + HumanInterface, return serialized first HumanPrompt.~~
  (Native verified via thread + recv. Wasm stub returns Err until D4.)

- [x] ~~**D3: tsot_apply_action FFI.**~~
  ~~Parse JSON HumanAction, push via action_tx, resume engine via Asyncify yield,~~
  ~~return next HumanPrompt as JSON. JS calls with `{async: true}`.~~
  (Native verified; wasm path returns Err until D4.)

- [x] ~~**D4: HumanInterface bridge (wasm-side).**~~
  ~~Save-and-replay shim works on native (`catch_unwind` + `YieldSignal`),~~
  ~~but wasm fails at link time — precompiled stdlib references~~
  ~~`__cxa_find_matching_catch_*` against the wrong exception ABI.~~
  ~~**Blocked on STATE_MACHINE.md S1-S6.** Native impl tested + ready;~~
  ~~wasm `_impl` functions return error stubs until S6 lands.~~
  (Resolved by STATE_MACHINE.md S6: `tsot_*_impl` now drive a
  `StepEngine` directly — no `catch_unwind`, no `panic_unwind` ABI
  dependency, identical code path on native and wasm. Save-and-replay
  scaffolding deleted along with `ScriptedSource` / `YieldSignal`.
  D4-era D2/D3 tests rewired and still green.)

- [x] ~~**D5: Port assets/play.html to WASM.**~~
  ~~Replace `fetch('/state'|'/action')` with `Module.ccall('tsot_*', ..., {async:true})`.~~
  ~~Load tsot_wasm.js via `<script>`. UI code (cards, click handlers) unchanged.~~
  (`<script src="tsot_wasm.js">` loads the emscripten loader; bootstrap
  awaits `createTsotModule()`. `wasmCallString` wraps `ccall` with a
  follow-up `tsot_free_string` so every `CString::into_raw` from the
  Rust side gets freed. `fetchState()` is now one-shot — runs
  `tsot_start_game` on first call with default args (random seed,
  50×blue-monkey mirror, heuristic opponent); subsequent calls return
  the stashed prompt since the engine only advances via
  `tsot_apply_action`. `sendAction` JSON-stringifies the
  `HumanAction`, parses the returned prompt JSON, stashes it on
  `state.current`. Asyncify is OFF (`.cargo/config.toml`) so the
  ccalls are synchronous — no `{async:true}` needed. UI / render code
  unchanged. `make serve` (HTTP shim) is broken until D8 retires it —
  the page now expects `tsot_wasm.js` alongside the HTML which the
  shim doesn't serve. D6 ships the static dev server that does.)

- [x] ~~**D6: Static dev-server scaffold.**~~
  ~~`make wasm-serve` target running `python3 -m http.server` from the dist dir.~~
  ~~COOP/COEP headers if SharedArrayBuffer later; not needed for v1 Asyncify.~~
  (Three Makefile targets: `make wasm` runs `cargo build --target
  wasm32-unknown-emscripten --release --bin tsot_wasm`, then stages
  `tsot_wasm.{js,wasm}` and `assets/play.html` → `index.html` into
  `dist/`. `make wasm-serve` depends on `make wasm` and then runs
  `python3 -m http.server $(WASM_SERVE_PORT)` (defaults to 8080) out
  of `dist/`. `make clean-wasm` removes `dist/`. Both gate on
  prerequisites: `emcc` for the build, `python3` for the serve. No
  COOP/COEP headers — SharedArrayBuffer / pthreads / wasm-workers
  remain disabled for v1 per `.cargo/config.toml`. `dist/` was already
  in `.gitignore`.)

- [ ] **D7: Smoke test in Chrome + Safari.**
  Play a full game vs UCT in both browsers. Verify payment / target picks /
  combat / activations / Main2 / game-over. Safari = future iOS WebView.

- [ ] **D8: Drop the HTTP shim.**
  Delete src/cli_serve.rs, the Serve subcommand wiring, the `make serve`
  target, and tiny_http from Cargo.toml. Replace with `make wasm`.

## Phase E — P2P multiplayer

- [ ] **E1: WebRTC data channel (JS-side).**
  Vanilla `RTCPeerConnection`. Two channels: ordered+reliable for actions,
  one for state-hash + keepalive. Public STUN server config.

- [ ] **E2: Pre-game commit-reveal handshake.**
  Both peers commit deck-hash + half-seed; reveal after exchange. Master seed
  = hash(seed_a, seed_b). Lower deck-hash plays side A (deterministic).

- [ ] **E3: AiKind::Remote variant.**
  New AiKind that reads opponent's actions from the data channel. Engine
  dispatch for that side blocks on the JS-bridged action source.

- [ ] **E4: Local action broadcasting.**
  Every local `tsot_apply_action` JS call also pushes the same action JSON over
  the data channel. Peer's engine consumes it via AiKind::Remote on opp side.

- [ ] **E5: State-hash drift detection.**
  Every 5 turns both clients broadcast hash(GameState). Mismatch → abort with
  diagnostic dump. Determinism work should mean this never trips.

- [ ] **E6: Signaling — manual copy-paste SDP.**
  v0: host clicks create-game, gets SDP-offer text, pastes into chat. Friend
  pastes answer back. No server infra. Ugly UX but proves end-to-end.

- [ ] **E7: Signaling — minimal relay server.**
  ~200 lines Node/Rust on any free VPS. WebSocket; peers exchange SDP via
  short room codes. No accounts, no game storage. Self-hostable.

- [ ] **E8: Reconnect / abort UX.**
  Data channel drops → "wait 60s / abort + claim win". Reconnect replays the
  action stream from the start; both clients re-derive state independently.

- [ ] **E9: Document hidden-info threat model.**
  RULES.md or MULTIPLAYER.md: both clients simulate fully → modded renderer
  sees opp hand. Deck-hash commits stop swap cheating. Mental poker = future.

## Phase F — mobile wrap

- [ ] **F1: Capacitor scaffold.**
  `npx cap init` over the dist directory, add ios + android platforms.
  Same static bundle (wasm + html) ships to both stores.

- [ ] **F2: iOS build target.**
  Xcode signing, App ID, TestFlight pipeline. WKWebView runs the same wasm.
  Apple Developer account required ($99/year).

- [ ] **F3: Android build target.**
  Android Studio + gradle build of Capacitor wrapper. Internal Test Track.
  Google Play Developer account required ($25 one-time).

- [ ] **F4: Touch-friendly UI pass.**
  Larger tap targets, no hover-only affordances, gesture-friendly. Test on
  actual iPhone + Android. Card text scaling for small screens.

- [ ] **F5: Mobile WebRTC verification.**
  Cellular NATs need TURN relay (~30% of connections). Decide: self-host
  coturn, paid service (Twilio/Xirsys), or accept failed connections.

## Phase G — polish

- [ ] **G1: localStorage save/load.**
  Reuse the JSON round-trip from tests/save_load.rs. FFI `tsot_serialize_state`
  / `tsot_load_state`; JS persists to localStorage. Resume-on-tab-reopen.

- [ ] **G2: WASM size optimization.**
  After full engine wasm is built (~3-5MB?), wasm-opt -Oz, strip debug,
  inspect biggest contributors. Lua C runtime probably dominates.

- [ ] **G3: Docs update for wasm-first path.**
  README.md: `make serve` → `make wasm`. LUA.md: emscripten path is now live.
  JOURNAL.md: cross out "multiplayer rollback netcode" once E5 lands.

- [ ] **G4: Decision log — Asyncify vs step-mode.**
  Note in JOURNAL.md (or new ADR): chose Asyncify for v1 (~1 day, ~50% bloat).
  Step-mode revisit if binary > 10MB on mobile or runtime overhead is visible.
