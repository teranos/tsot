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
  (Shipped as the Worker model — see G4 for why-not-Asyncify. Main
  thread spawns `new Worker('tsot-worker.js')`; the worker
  `importScripts('tsot_wasm.js')` and `createTsotModule()` instantiates
  the wasm in worker scope. FFI is async via `postMessage`:
  `{cmd: 'start_game' | 'apply_action', ...}` outbound, the worker
  ccalls synchronously and posts `{kind: 'envelope', json}` back; main
  resolves a single `pendingFfi` Promise per call (engine is
  single-threaded, only one inflight FFI). `wasmCallString` lives
  inside the worker and frees every `CString::into_raw` via
  `tsot_free_string`. Asyncify is OFF in `.cargo/config.toml`; the
  ccalls inside the worker are still synchronous — the async surface
  is the postMessage round-trip on the main side. Live observability
  arrives mid-FFI: `--js-library=assets/wasm-worker-lib.js` resolves
  the `tsot_emit_iteration_event` extern in `src/sim/uct.rs`, which
  posts `{kind: 'uct_iter', line}` from worker → main once per UCT
  iteration; main renders `[live UctIter]` lines into the LOG while
  the search is still hanging. (Worker + async FFI verified
  end-to-end on Firefox: page loads, `start_game` round-trips,
  hand renders. `[live UctIter]` mid-FFI rendering is shipped but
  not yet verified — needs a cast that triggers a real UCT search.
  Chrome + Safari are D7.) `fetchState()` runs `start_game` on
  first call with default args (random seed, 50-card varied deck,
  UCT opponent); subsequent calls return the stashed prompt since the
  engine only advances via `apply_action`. UI / render code unchanged.
  `make serve` (HTTP shim) is broken until D8 retires it — the page
  now expects `tsot_wasm.js` + `tsot-worker.js` alongside the HTML
  which the shim doesn't serve. D6 ships the static dev server that
  does.)

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

- [x] ~~**D7a: Smoke test in Firefox.**~~ (primary test target)
  ~~Play a full game vs UCT. Verify payment / target picks / combat /~~
  ~~activations / Main2 / game-over. `[live UctIter]` lines stream during~~
  ~~UCT search.~~
  (Closed 2026-06-06 — full game vs UCT played end-to-end in Firefox,
  reached game-over. Path 1 SharedArrayBuffer cancellation (H4) made
  preview responsive enough that the click-during-thinking case
  worked. No reproducible bugs observed in the run.)

- [ ] **D7b: Smoke test in Chrome.** (deferred)
  Same checklist as D7a, in Chrome. Deferred — Firefox is the active
  test target. Picked up if a Chrome-specific regression is reported
  or once mobile work (F-phase) makes Chromium parity load-bearing.

- [ ] **D7c: Smoke test in Safari.** (deferred — iOS WebView proxy)
  Same checklist as D7a, in Safari. Deferred — Firefox is the active
  test target. Picked up when iOS build work (F2) starts; Safari is
  the desktop proxy for WKWebView so its result gates F2 confidence.

- [x] ~~**D8: Drop the HTTP shim.**~~
  ~~Delete src/cli_serve.rs, the Serve subcommand wiring, the `make serve`~~
  ~~target, and tiny_http from Cargo.toml. Replace with `make wasm`.~~
  (Narrowed scope: HTTP shim cluster only. Deleted `src/cli_serve.rs`,
  `mod cli_serve` + `use cli_serve::ServeArgs` + `Serve(ServeArgs)`
  variant + dispatch arm from `src/main.rs`, `tiny_http = "=0.12.0"`
  from `Cargo.toml`, `serve:` target + its doc comment from `Makefile`,
  `serve` from the .PHONY list. `make serve` no longer exists; the
  browser entry point is `make wasm-serve`. `run_game_continue` is
  still `#[deprecated]` and reached by `cli_matchup_mcts`,
  `sim::run::run_game_with_ai`, and two tests — left in place; the
  day no caller exists it deletes itself.)

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

- [x] ~~**G1: localStorage save/load.**~~
  ~~Reuse the JSON round-trip from tests/save_load.rs. FFI `tsot_serialize_state`~~
  ~~/ `tsot_load_state`; JS persists to localStorage. Resume-on-tab-reopen.~~
  (Superseded by H2 — went IndexedDB instead of localStorage. FFI
  shipped as `tsot_save_game` / `tsot_load_game` carrying SaveFile JSON
  (state + EngineCursor). JS persists to IndexedDB `tsot.saves` keyed
  by auto-increment id with `{name, savedAt, json}`. Save / Load saved
  / Download / Load file buttons in `assets/play.html` cover both
  in-browser persistence and the dev-loop bug-report round-trip.
  IndexedDB rather than localStorage because saves run hundreds of KB
  with full GameState + journal, well past localStorage's ~5MB string
  quota and synchronous-API ceiling; tab-reopen resume works for free
  off the same store.)

- [ ] **G2: WASM size optimization.**
  After full engine wasm is built (~3-5MB?), wasm-opt -Oz, strip debug,
  inspect biggest contributors. Lua C runtime probably dominates.

- [ ] **G3: Docs update for wasm-first path.**
  README.md: `make serve` → `make wasm`. LUA.md: emscripten path is now live.
  JOURNAL.md: cross out "multiplayer rollback netcode" once E5 lands.

- [x] ~~**G4: Decision log — Web Worker for wasm FFI.**~~
  ~~Note in JOURNAL.md (or new ADR) comparing Web Worker against the~~
  ~~yield-based alternatives (Asyncify, JSPI). Comparison table +~~
  ~~costs accepted + references.~~
  (Shipped as `docs/adr/0001-web-worker-for-wasm-ffi.md`. ADR
  centers on Web Worker as the chosen mechanism; includes the 5-row
  comparison table against JSPI + Asyncify; documents why neither
  yield-primitive fits — both stay single-threaded so an 80s UCT
  search still freezes the UI, and JSPI specifically has no mobile
  reach as of June 2026.)
