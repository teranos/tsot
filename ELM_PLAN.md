# tsot — Elm dev-tool migration plan

> Update by crossing through (`~~task line~~`) when done. ≤3 sentences per stage.

Port the in-browser dev tool from inline JS in `assets/play.html` into
typed Elm under `assets/src/`. Destination: `play.html` is markup + worker
bootstrap only; all UI + state in Elm; the JS bridge is ~50 lines of
generic envelope forwarding.

- [x] ~~**1: Pipeline + Elm:ready pill.**~~
  ~~`elm make` integrated into `make assets`. `play.html` mounts
  `<div id="elm-root">` and renders a green "Elm: ready" pill.~~

- [x] ~~**2: Build-info footer.**~~
  ~~First JS→Elm port `buildInfoIn` carries `window.__TSOT_BUILD__`.
  Elm renders the bottom-right footer.~~

- [x] ~~**3: LOG panel.**~~
  ~~`logTextIn` + `logErrorIn` ports. Elm owns the LOG container —
  text lines + styled `.log-error` blocks with breadcrumbs / JS
  stack / raw stderr.~~

- [x] ~~**4: Decision-report panel.**~~
  ~~First Elm→JS port (`decisionFetchOut`) — both port directions
  exercised. Elm owns the panel state machine + per-card aggregator;
  js-bridge owns the `decision_log` IDB read.~~

- [x] ~~**5: Saved-list panel.**~~
  ~~Per-row Load/Download/Delete buttons in Elm. Introduces
  records-as-port-arguments and JS-initiated refresh through the
  same inbound port (`SavedListReceived` only paints if visible).~~

- [x] ~~**6: Save-controls bar.**~~
  ~~All eight buttons + save-status span move into Elm.
  `gamePhaseIn` carries `state.phase` transitions so Save/Download
  disable themselves when not playing.~~

- [x] ~~**7: Deckbuilder.**~~
  ~~First attempt failed and was reverted; landed as stage 10 on the
  collapsed bridge. See note below.~~

- [x] ~~**8: Fault-surface diagnostic.**~~
  ~~Wrap the js-bridge IIFE in `try/catch` with per-block `stage`
  markers. Throws inject a fixed-position red banner with stage +
  message + stack; success logs `app.ports=[…]` into the LOG.~~

- [x] ~~**9: Collapse the bridge.**~~
  ~~Nine per-feature outbound ports collapsed to two generic envelope
  ports (`workerCmdOut`, `idbReqOut`). Future stages dispatch by cmd
  string instead of port name; unknown cmd/op surfaces via the fault
  banner.~~

- [x] ~~**10: Deckbuilder (retry on collapsed bridge).**~~
  ~~Same scope as stage 7 — pool/filters/deck/preset/AI/Start/Spectate
  in Elm. workerCmdOut extended to `{cmd, payload}` envelope; one
  `bootDataIn` inbound port for the startup card-pool + presets push.~~

- [~] **11: Game-screen render.** Biggest remaining island. Split
  into substages because each piece is independently verifiable and
  the visual relocation needed a layout fix mid-stream.

  - [x] ~~**11a**: meta line via `gameMetaIn` (parallel with the JS-
    rendered `#meta` to verify the port).~~
  - [x] ~~**11b**: drop the JS meta render — Elm is the sole renderer.~~
  - [x] ~~**11c**: `#prompt` line via `promptTextIn`. The 24
    `document.getElementById('prompt').textContent = …` sites all
    route through a single `setPrompt(text)` helper.~~
  - [x] ~~**11d**: `gameStateIn` carries the full `{state, prompt}`
    envelope on every `_renderInner` call. Stored raw as `Model.gameState
    : Maybe D.Value` — no top-level decoder; subsequent substages
    pull slices out at the view site.~~
  - [x] ~~**11e**: move `<div id="elm-root">` above the JS-controlled
    `#game-screen`. Page-level Elm chunks now render in their natural
    top-of-page position; inside-`#game-screen` items still displaced
    until 11f.~~
  - [x] ~~**11f**: scaffolding port — Elm renders the `#game-screen`
    zone wrappers (rows, zones, headers, empty `.cards` containers).
    Container IDs preserved so JS `appendChild` still works. Scaffold
    renders unconditionally on `gamePhase` = Playing/Spectating (the
    gameState gate caused a timing race — `_renderInner` ran
    synchronously after `setPhase`+`await` and reached for
    `opp-board-cards` before Elm's first paint with `Just gameState`;
    fix was to drop the gate, counts/deck-tops fall back to
    placeholders when no slice has landed). The three `style.display
    = …` toggles in load/spectate/start are gone.~~
  - [x] ~~**11g**: per-player counts rendered from `Model.gameState`
    (opp-counts `deck:N hand:N ex:N`, opp-gy-count, your-gy-count,
    your-hand-counts `deck:N ex:N`). The four `_renderInner`
    textContent writes are gone.~~
  - [x] ~~**11h**: deck-top displays (back-of-card colors+symbols).
    `renderDeckTop` + 4 call sites deleted; `viewDeckTop` is the sole
    renderer.~~
  - [ ] **11i**: opponent board (read-only — requires `cardEl`
    ported to Elm).
  - [ ] **11j**: opponent + your graveyards (uses `cardEl`).
  - [ ] **11k**: your board (still read-only render; click handlers
    arrive in 11n).
  - [ ] **11l**: your hand (read-only render).
  - [ ] **11m**: `#buttons` div (Pass / Confirm / Cancel) — click
    handlers fire FFI actions through `workerCmdOut`.
  - [ ] **11n**: prompt-kind branches — PickCard / PickAttackers /
    PickBlocks / ChooseCard / Confirm / ChoosePlayer / ChooseInt /
    GameOver / Spectate / Activations / Main2Pick. Click semantics
    + state for each. Probably split further.
  - [ ] **11o**: UCT preview status + casting banner.

- [x] ~~**12: Spectator bar.**~~
  ~~First module split out of `Main` — `SpectatorBar.elm` owns Model
  + Snapshot + Config-record + view + decoder. Pattern: Msg
  constructors live in Main (passed via `Config msg`), no sub-update,
  state stays in `Model.spectatorBar`. Bar visible only when
  `active=true`. JS still owns the snapshots array + `setInterval`
  handle; on every state change (seek/step/play tick/pause/speed/exit)
  JS pushes a `{active, index, total, playing, msPerStep, winner,
  snapshot}` projection via the new `spectatorStateIn` port. The seven
  clicks each fire a `workerCmdOut` envelope with a `spec_*` cmd; the
  bridge dispatches to `window.tsotSpec*` shims. The previous
  `<div id="spectator-bar">` DOM + `wireSpectatorBar` + the eight
  `getElementById('spec-*').onclick=…` wirings are gone.~~

- [ ] **13: workerCall + state to Elm.**
  Remaining inline-JS infra (`workerCall`, render loop, `recordDecision`
  write path) ports to Elm. play.html shrinks to the worker bootstrap
  — under 100 lines, no UI logic.

- [ ] **14: Cleanup.**
  After 13, JS-side state mirrors and transitional `window.tsot*`
  helpers have no callers. Delete them and verify the tool runs on
  Elm + the thin bridge alone.

## Note on stage 7

The first attempt at the deckbuilder failed with
`window.tsotCardPoolReceived is not a function` and was reverted. Root
cause was structural: the js-bridge IIFE had no fault surface, so a throw
at any line silently killed every shim it would have set up afterward —
the failure surfaced N indirections later as a useless TypeError at a
downstream consumer in `play.html`. Stages 8 (fault surface) and 9
(collapse bridge) address the structural problem; stage 10 is the
deckbuilder retry on the simpler surface.
