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
  - [x] ~~**11i–11o (single lift):**~~ `cardEl` + four card zones +
    `#buttons` + every prompt-kind branch + UCT preview port wiring +
    casting banner — landed across 5 sub-waves into `GameScreen.elm`.
    Originally planned as seven separable substages; cleavage analysis
    proved they can't be split — every prompt-kind branch
    (`PickAttackers` / `PickBlocks` / `ChooseCard` / etc.) re-fills
    the SAME card containers with prompt-specific click handlers, so
    "read-only zones first" was impossible. Waves landed as:
    Wave 0 (foundation — types, decoders, `viewCard` primitive),
    Wave 1 (Confirm / ChoosePlayer / ChooseInt buttons via the new
    `#elm-prompt-buttons` sibling of JS-owned `#buttons`),
    Wave 2 (all 5 card containers Elm-rendered read-only via
    `viewCard`; ~280 lines of JS `appendChild(cardEl)` deleted),
    Wave 3+5 combined (PickCard hand+board interactivity + Pass
    button + ChooseCard pool/host targeting + Skip button; promoted
    Wave 5 from last to second-last because hand-payment casts were
    blocked without it), Wave 4 (PickAttackers + PickBlocks click
    handlers + the staging-dance state machine, the first work
    landed test-first via the new `elm-test` infrastructure).
    UCT preview rendering (Wave 5 in the original plan) — port
    wired but kickoff still JS; tagged separately under stage 13.

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

## Module splits (parallel track)

`Main.elm` is the single source of `Model` + `Msg` + `update` + ports
+ subscriptions. Render-only modules get extracted as they become big
enough to be worth their own file; the pattern is consistent:

- Module owns: types it renders, decoders for its inbound port
  envelopes, view function(s), any module-local helpers.
- Module is Msg-agnostic; click handlers take a `Config msg`
  constructor record that `Main` provides with its concrete `Msg`.
- State lives in `Main.Model` as a field of the module's type.
- Port wiring + Msg dispatch + `update` branches all stay in `Main`.

Splits to date (LOC moved out of `Main` shown):

- [x] ~~**SpectatorBar.elm**~~ — ~236 lines. First split; established
  the `Config msg` pattern.
- [x] ~~**LogPanel.elm**~~ — ~186 lines. Pure render of LOG entries
  (text + error blocks); exports `containerId` so Main's
  scroll-to-bottom Cmd targets the same id.
- [x] ~~**BuildFooter.elm**~~ — ~77 lines. Tiny; sets the precedent
  that even small islands earn a file once Main pushes past ~2k LOC.
- [x] ~~**GameScreen.elm**~~ — ~900 lines. Game-screen render path
  (CardView + Activation + Prompt ADT + UctCandidate / UctPreview +
  CardOpts primitive + viewCard + viewPromptButtons + CombatSelection
  state machine + decoders for every prompt variant). Largest module
  to date; landed as the host for the chunk B big lift rather than
  as a Main extraction.

## Tests

CLAUDE.md mandates "Strict TDD required". `elm-test` is wired
(`assets/elm.json` + `assets/tests/` + `flake.nix` provides
`elmPackages.elm-test`). `make assets` build green AND `elm-test`
green is the bar before any commit. Current suites:

- [x] ~~**GameScreenTest.elm**~~ — 12 tests: `decodeCardView` full
  shape, every `decodePrompt` variant, `viewCard` DOM-class behavior
  under different `CardOpts`.
- [x] ~~**CombatSelectionTest.elm**~~ — 12 tests pinning the
  PickAttackers + PickBlocks state machine: `toggleAttacker`,
  `clickBlocker` (stage / unstage / unassign / re-stage),
  `assignAttackerToStaged` (assignment + clearing + gang-block),
  `resetCombatSelection`. Failing-test-first then implementation.

Remaining easy splits (rough order):

- [ ] **SavedListPanel.elm** — ~100 lines. Self-contained state
  machine + per-row Load/Download/Delete buttons.
- [ ] **SaveControls.elm** — ~120 lines. Top button bar (Save /
  Download / Load file / Test panic / Load saved… / Decision report /
  Export / Clear) + save-status span.
- [ ] **DecisionPanel.elm** — ~250 lines. Aggregator + table render;
  largest of the easy splits.
- [ ] **Deckbuilder.elm** — ~400 lines. Biggest standalone island;
  pool + filters + deck + preset + AI pickers + Start/Spectate.
  Lands once the smaller splits validate the pattern under load.
- [ ] **GameScreen.elm** — chunk 11i–11o lands as a new module from
  the start rather than landing in Main and being split later.

## Note on stage 7

The first attempt at the deckbuilder failed with
`window.tsotCardPoolReceived is not a function` and was reverted. Root
cause was structural: the js-bridge IIFE had no fault surface, so a throw
at any line silently killed every shim it would have set up afterward —
the failure surfaced N indirections later as a useless TypeError at a
downstream consumer in `play.html`. Stages 8 (fault surface) and 9
(collapse bridge) address the structural problem; stage 10 is the
deckbuilder retry on the simpler surface.
