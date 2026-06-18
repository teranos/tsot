# tsot — Sacred Error Axiom

## The axiom

**An Error is a first-class primitive — one typed value that crosses
every layer of the system unchanged.** Where a `Card` is the entity
the game manipulates, an `Error` is the entity any layer emits when
something goes wrong. Both get the same architectural treatment:
their own module, their own render path, their own CSS surface,
their own contract for how they cross system boundaries.

Restating the principle in CLAUDE.md's language: *errors are sacred,
first-class citizens, never collapsed, dropped, swallowed or
suppressed; they land in front of the user, contextually at points
of interaction.* Today the codebase honors this only in scattered
ad-hoc surfaces (panic envelope in `js-bridge.js`, `LogPanel.ErrorEntry`
in Elm, `eprintln!` in Rust). An Error primitive unifies them.

## Why Error needs its own module

By analogy to `Card.elm`:

| Card | Error |
|---|---|
| `Card.elm` — single render path, decoder, key, styles | `Error.elm` — single render path, decoder, key, styles |
| `Card.view` returns ONE DOM element per iid | `Error.view` returns ONE DOM surface per Error id |
| `Card.decode` reads the engine wire shape | `Error.decode` reads any failure wire shape (port payload, FFI envelope, panic envelope) |
| `Card.styles` owns the visual contract | `Error.styles` owns the error visual contract |
| `CARD.md` — axiom + roadmap | `ERROR.md` — this document |
| Violation = "the same card looks different in two surfaces" | Violation = "the same failure surfaces differently in two places, or doesn't surface at all" |

## What the axiom forbids

1. **Silent drops.** No `Err _ -> (model, Cmd.none)`, no `.catch(() => {})`,
   no `let _ = result` on a `Result` that carries meaningful failure,
   no `unwrap_or_default` on `Result`, no `eprintln!`-and-continue
   without the Error pipeline being notified.
2. **Stringly-typed error envelopes.** A failure crossing a layer
   boundary must be the `Error` type, not a free-form `String`. Strings
   rot; typed values type-check.
3. **Ad-hoc per-surface error renders.** No bespoke "show this red"
   div in every component — the Error primitive's `view` function is
   the one render path, just like `Card.view` is the one render path
   for cards.
4. **Errors that are visible only in browser devtools.** CLAUDE.md is
   explicit: errors land in front of the user, contextually. Devtools
   are an admission of failure to surface.
5. **Errors without context.** An Error without `where it originated`
   (which surface, which interaction, which payload) is unactionable.
   No `"decode failed"` without the field path + raw input.

## Unwired — current inventory (2026-06-16)

Sacred-errors is the project's non-negotiable axiom (`CLAUDE.md`).
This section enumerates every known gap so future sessions don't
re-grep to rediscover the work. Tick a box only when the gap is
closed AND user-verified end-to-end (a commit means done — see
`CLAUDE.md`).

### Hot — panics / active silent drops

- [~] `src/sim/step/combat.rs:116` — `self.state.confirm_attacks().unwrap()`
  **Shipped 2026-06-16**: replaced with match-on-Err that routes
  through `emit_human_refusal` (surface=`"prompt"`, region=
  `"confirm-attacks"`). Cursor advances to DeclareBlockers anyway
  so the game doesn't deadlock. Not yet user-verified end-to-end
  (needs a real refusal to fire).
- [~] `src/sim/step/combat.rs:234` — `self.state.confirm_blocks(...).unwrap()`
  **Shipped 2026-06-16**: same shape, region=`"confirm-blocks"`.
  On Err, skips outcome accounting (no mills/deaths credited) and
  advances cursor. Not user-verified.
- [~] `step_resolve` Err-arm in `src/sim/step/main_phases.rs` missing
  `ChoicePending` intercept. **Shipped 2026-06-16**: intercept
  added (mirror of `main_phases.rs:~439`); Pending now rolls back
  preview journal, sets cursor via `ctx.on_pending(picked, history)`,
  yields `NeedHuman(pending_to_prompt(...))`. The lying
  `play_error_user_message::ChoicePending` arm rewritten to a
  meta-message pointing at the call-site fix. Read the Embers'
  on_play game.choose_card should now suspend → prompt → resume.
  Not user-verified end-to-end.
- [x] ~~9 `.is_ok()` discard patterns audited — 6 legitimate
  (test assertions, AI/EA paths, already-wired step_resolve sites);
  0 are sacred-error violations~~ (2026-06-16).

### Wrong-diagnostic lies currently shipping

- [ ] `play_error_user_message::ChoicePending` message reads
  "ChoicePending escaped the Main2 catch arm. File a bug." — the
  bug is a MISSING intercept, not an escaped one. Future-you goes
  hunting for a leaky catch arm that doesn't exist.
- [ ] `play_error_user_message::VariableXValueMissing` reads
  "The UI needs to ask first." — blames the UI for an engine
  wiring gap (no NeedHuman X-prompt is yielded before play_card).
- [~] `crate::error::emit_region(..., format!("{e}"))` in
  `src/game/lua_api.rs` (3 sites) does NOT walk mlua's
  `CallbackError { cause: WithContext { cause: ExternalError(...) } }`
  chain. Outer wrapper text shown; inner Lua line:message hidden.
  `pending_from_mlua_error` already walks the chain — adopt the
  same walk for the surfaced `why` field.
  **Code shipped 2026-06-18** (NOT user-verified end-to-end): added
  `mlua_error_chain_why()` next to `pending_from_mlua_error()` in
  `src/game/lua_api.rs`; same chain traversal (`CallbackError`
  stripped, `WithContext` context emitted, leaf RuntimeError/
  SyntaxError preserved with line:message), layers joined with ` → `.
  The 3 `emit_region` sites in `fire_self_only`, `fire_activated`,
  `fire_with_partner` now pass the walked string instead of
  `format!("{e}")`. 5 unit tests pin the contract: leaf renders with
  line preserved; CallbackError wrapper stripped; WithContext emits
  context + cause; nested chain walks fully; non-RuntimeError leaves
  (SyntaxError) still render via Display. Verification debt: trigger
  a real handler failure in the dev tool and confirm the inner Lua
  line:message appears in the error overlay's `why` field.
- [ ] `TRACE_STRING_ALLOWLIST` (in `src/trace.rs`) catches NEW
  bare-String fields but doesn't audit the 14 existing entries
  (`Step.from/to/result`, `Cursor.from/to`, `Oracle.call/answer`,
  `Winner.cause`, `Ffi.span`, `AiPick.ai`, `Error.source/message`,
  `Count.key`, `Handler.event`). Each could be conflating two
  meanings today the way `outcome: String` did before OutcomeRepr.

### Engine internals (medium)

- [~] 32 `eprintln!` sites in non-CLI Rust still surface only to
  stderr (invisible in browser). `grep -rn 'eprintln!' src/
  --include='*.rs' | grep -v cli_ | grep -v _tests`.
  **Triaged 2026-06-18**: of the original 32, the actual
  failure-surface set (sites where a real engine bug should land in
  the dev tool) is smaller. Triage:
  - **3 lua_api.rs handler-failure sites** (lines 1435, 1483, 1588):
    already typed-Error since 2026-06-11; eprintln kept as native-CLI
    fallback (intentional). Improved 2026-06-18 with chain walker
    (see "wrong-diagnostic lies" item above).
  - **3 sites migrated 2026-06-18** (code shipped, NOT user-verified
    end-to-end): `src/game/play.rs:976` `[CHAIN OVERFLOW]` →
    `Severity::Error` `surface="engine" region="response-stack-overflow"`;
    `src/game/play.rs:1010` `[RESPONSE SPIN]` →
    `Severity::Error` `surface="engine" region="response-spin"`;
    `src/sim/mcts.rs:301` unexpected ChoicePending →
    `Severity::Error` `surface="mcts" region="unexpected-pending"`.
    eprintln kept as native-CLI fallback in each.
  - **~20 sim/run.rs EA-diagnostic sites** (heartbeat, slow-cast,
    state dumps, baseline-load errors): EA binary is native-CLI only;
    these are intentional terminal progress reporting per the
    `cli_*.rs` exemption rule (sim/run.rs isn't `cli_*` but its only
    consumer is the EA CLI binary).
  - **2 trace.rs sites** (lines 410, 540): cfg-gated
    `#[cfg(not(target_arch = "wasm32"))]` native-only fallbacks
    paired with wasm-side `tsot_emit_info` / `tsot_emit_panic`
    externs. Exempt — wasm path already surfaces.
  - **`game.print()` Lua debug** (lua_api.rs:1176): intentional
    dev-tool debug print. Exempt per `ERROR.md` original guidance.
  - **main.rs:136, 146**: CLI startup. Exempt.
  Net remaining failure-surface sites needing migration: zero in
  this codebase as of 2026-06-18. If a future Rust site adds an
  `eprintln!` for a real failure, that site must route through
  `crate::error::emit_region` BEFORE the eprintln fallback.
- [ ] 97 `let _ = result;` patterns in non-test code — most are
  setup ("genuinely can't fail here"), ~20% need to either route
  through the pipeline or carry a one-line `// silent because …`
  justification.
- [ ] `activate_ability` Err paths outside `src/sim/step/`.
- [ ] `src/card/loader.rs` malformed-card handling — does a
  rejected card surface anywhere, or does the corpus silently
  shrink by one entry?

### JS

- [ ] 16 of 46 `catch (...)` blocks in `assets/` still silent or
  routing only through the legacy `buildErrorBlock` path. Audit:
  `grep -rn 'catch\s*(' assets/ --include='*.js' --include='*.html'`.
- [ ] **ERROR.md Slice 1 deferred bullet, still open:** `LogPanel.ErrorEntry`
  → `Error.view` collapse. Two parallel error renderers exist;
  `appendErrorEvent`/`buildErrorBlock` (LogPanel) and `tsotPushError`
  (Error.view). Same failure surfaces differently in two places —
  the axiom violation rubric calls this out explicitly (§ Why
  Error needs its own module, last row).

### Elm

- [ ] 53 `Maybe.withDefault` + 3 `Result.withDefault` — triage
  needed. Most are legitimate "this Maybe carries 'absent' not
  'failed'" defaults; ~10-15 are failure-swallow patterns that
  should route through `pushDecodeError`.
- [ ] `LogPanel.elm`, `GameScreen.elm`, `SpectatorBar.elm`,
  `BuildFooter.elm` — unsweept. Only `Main.elm` has the 7 typed
  decode-error sites today.

### Engine-correctness bugs surfaced by play (not sacred-errors gaps,
### but their invisibility before sacred-errors made them hide)

- [x] ~~`game.damage(iid, n)` from Lua never invoked B.8 death check.
  A 2/2 taking 2 damage from Read the Embers stayed on the board.
  **Fixed 2026-06-16**: extracted `cleanup_b8_damage_deaths()` in
  `play.rs`, called from `do_damage` after `set_damage`. Two tests
  pin the contract: damage ≥ Y kills, damage < Y survives.~~
- [x] ~~`game.damage(target, n)` only accepted creature iids.
  Cards saying "deal N damage to any target" couldn't actually
  target the opponent player (Read the Embers had a TODO comment).
  **Fixed 2026-06-16**: `do_damage` now detects "a"/"b" target as
  player id; mills N from their DECK to EXILE (RULES B.2 analog,
  L.1 loss check). Read the Embers' Lua handler updated to include
  the opponent in the choice pool. One test pins it.~~

### Architectural gaps (need engine work, not just wiring)

- [ ] **Graveyard payment human choice.** When a cast has a GY cost
  source and the player has MULTIPLE color-anchor-satisfying cards
  in their graveyard, `resolve_graveyard_payment` picks
  deterministically — the human never gets to choose which card
  pays. Fine for AI rollouts; wrong for human agency. Slice: add
  `NeedHuman(ChooseCard)` for GY payment when active is Human,
  same shape as the existing HAND-payment human pickers.

- [ ] **Variable-X cast-time prompt.** No engine path yields a
  `NeedHuman(ChooseInt)` for X before `play_card` runs. The
  `oracle.choose_int` inside `build_pattern_b_choices` is the
  closest existing path; making it work for human-active casts
  is the slice. Until then Read the Embers (and every X-cost
  card) can only be cast via the Lua-yield workaround if at all.
- [ ] **Cast-time targeting** for spells with declared targets
  (Fireball, every "target a creature" spell). Card schema needs
  a `target: Option<TargetSpec>` field; engine yields a
  `NeedHuman(ChooseCard)` BEFORE handing to `on_play`; R.1.a
  response window fires with the target locked. Lua reads
  `game.cast_target()`. Closes the Fireball "Y/N then choose"
  workaround pattern.
- [ ] **Activation flow through Main1/Main2.** Engine currently
  surfaces a typed Error saying activations aren't supported
  (sacred-errors win), but doesn't yet route a clicked
  activation through the cursor/oracle path. Signal Goblin,
  jewel hand-pay, etc. block on this.

### Self-enforcement holes

- [ ] `every_step_file_references_emit_human_refusal`
  (`src/sim/step/tests.rs`) only covers `src/sim/step/`. Extend to
  `src/game/lua_api.rs`, `src/game/play.rs`, `src/wasm_ffi.rs`,
  `assets/play.html`, `assets/js-bridge.js`, plus the Elm modules
  via a regex-based source scan in `elm-test`.
- [ ] No CI grep that fails the build when a PR introduces a new
  silent-drop pattern (`let _ = ` on a meaningful Result, bare
  `catch`, `Err _ ->` in Elm). Counterpart to the allowlist test.
- [ ] No Elm equivalent of `TRACE_STRING_ALLOWLIST` for ports —
  port-payload shapes can drift without a test catching the case
  Main.elm's 7 sites have to defend against today.

### Verification debt (shipped but not user-confirmed end-to-end)

- [ ] Build watermark visible on every Error window across all
  surfaces (only confirmed on Test Panic + the Read the Embers
  refusal so far).
- [ ] Read the Embers cast actually completes once `step_resolve`
  ChoicePending intercept lands.
- [ ] Spectate path error surfacing under a real failure.
- [ ] Save / Load error surfacing under a real failure.
- [ ] Deckbuilder boot error surfacing on a deliberately broken
  preset (the `build_preset_decks` validator emits a typed Warn —
  needs a malformed preset to fire end-to-end).

### Doc / axiom open items

- [ ] ERROR.md Slice 6 — `localStorage` persistence + bijectivity
  invariant for `Error.id` → DOM node.
- [ ] OBSERVABILITY.md Phase 2 — AI-internals narration (O6, O7,
  O8) so UCT opponent reasoning surfaces.
- [ ] OBSERVABILITY.md Phase 5 — UI filter chips + click-to-expand
  for the LOG so the trace stream is navigable, not a wall.

## Visual contract

The Error render is **not a side panel, not a drawer, not a LOG line
the developer has to scroll a panel to find**. It is an **overlay
anchored at the surface where the failure originated**, styled as a
terminal-style diagnostic block:

- **Position — the primary case is the cursor anchor.** Developer
  mental model: *"I click on something, it doesn't work, error right
  there where my cursor is."* So the overlay opens AT the cursor's
  position at the moment the failing action fired, not at the
  surface's bounding box. The Error carries an `Anchor { x, y }` in
  its `context` captured from the DOM `MouseEvent.clientX/clientY` of
  the interaction. Smart fallback if the anchor would clip off-screen.
- **Position — fallback for port-triggered failures.** Errors that
  weren't triggered by a click (async port payload comes in and fails
  to decode — buildInfo, gameMeta, spectator state, panic envelope)
  have no cursor position. They fall back to anchoring at the
  surface's bounding box via `context.surface` + `context.region`.
  These are the minority case; the majority of operator-visible
  failures come from clicked actions.
- **Background**: dark red (≈ `#2a0c0c` filled, `#4a1414` border).
  Saturated enough to be impossible to miss without being neon
  unreadable. CSS lives in `Error.styles`.
- **Typography**: monospace (`ui-monospace`, matches the existing
  terminal aesthetic of the dev tool). Pre-formatted text, white-space:
  pre-wrap so multi-line stack content reads naturally.
- **Coloring inside the block**:
  - severity ribbon (left edge stripe): info `#88f`, warn `#fc6`,
    error `#f66`, panic `#f0f`.
  - field labels (e.g. `error:`, `why:`, `trace:`) in a muted
    foreground (`#888`).
  - error message in the severity color.
  - why / cause chain in a lighter foreground (`#ddd`).
  - trace (call stack, field path, originating cursor) dimmed (`#aaa`)
    and indented one level so the eye can skip it when scanning.
- **Content** (in this order, top to bottom):
  1. `error: <one-line title>` — e.g. `error: bootDataIn decode failed`.
  2. `why: <human reason>` — the developer-actionable chain. For a
     decoder failure: `JSON path` + `expected` + `got`. For an FFI
     failure: which call + which arg. For a Lua error: the cards/
     filename + line.
  3. `trace:` — the structured TraceEvent chain leading up to the
     failure, drained from the OBSERVABILITY bus at the moment of
     emission. The developer reads down the trace to see what the
     engine was doing just before the error fired.
  4. `dismiss [esc]` affordance in the bottom-right corner, tiny.
- **Dismissal**: clicking the dismiss affordance or pressing `Esc`
  removes the overlay. The Error stays in the persistent error
  log (Slice 6 — `localStorage`) so it's recoverable. Dismissal is
  a state transition, not a destroy — same DOM element, just
  `display: none`.

This is the SOLE render for an in-flight Error. The LOG-as-fallback
view is a stripped historical mirror (no overlay, just a line per
past Error in the timeline) for traceability across a session, not
for live attention.

## What the axiom requires

1. **One Error type.** `Error.elm` defines `Error` as a record with
   stable identity (`id`), severity, originating context (which
   surface / region), human title, machine detail, optional raw
   payload sample, timestamp. Same shape on the JS side and the Rust
   side — serde + Elm decoders agree byte-for-byte.
2. **One render path.** `Error.view` is the canonical renderer. The
   LOG panel uses it. Contextual inline surfaces use it. The panic
   banner uses it. Any new component that wants to display an error
   imports `Error.view` rather than rolling its own div.
3. **Routed at every boundary.** Every port decoder, every `Result`
   propagation, every JS catch, every Rust `eprintln!` site routes
   through the Error pipeline. The bottom of the pipeline is a single
   port (Elm-side `errorIn` + `errorOut`) and a single Rust enum
   variant on the FFI envelope (`{ ok, err, prompt, log, trace, errors }`).
4. **Contextual placement.** An Error declares its `context` (surface
   - region) at construction. The renderer reads `context` and places
   the surface at that point of interaction — the deckbuilder dropdown
   shows decode failures inline; the prompt bar shows handler failures
   inline; the LOG shows everything as a fallback timeline.
5. **Stable identity.** Each emitted Error has an `id` (UUID or
   monotonic counter scoped to a session). The same Error rendered in
   the LOG and inline at its surface is keyed to the same DOM node
   per the Card-axiom analog — `Html.Keyed` over Error.id where the
   primitive renders in a list.

## Roadmap

Numbered slices, smallest to largest. Mark `[x] ~~slice~~` when
shipped end-to-end (per CLAUDE.md commit standard).

### Slice 1 — the type + the renderer

- [x] ~~`Error.elm` lands with the `Error` record type, `Severity` enum,
  `Context` record (now carrying `Anchor` for cursor-position
  capture), `decode`, `clickAnchorDecoder`, `view`, `viewLogLine`,
  `styles`, `key`. Public API mirrors `Card`'s.~~
- [ ] `LogPanel.elm` switches its existing `ErrorEntry` variant to
  hold an `Error` and route through `Error.view`. The bespoke
  `.log-error*` styles in `play.html` move into `Error.styles`.
  (Deferred — `LogPanel.ErrorEvent` carries source-specific fields —
  `ffiCall`, `breadcrumb`, `jsStack`, `rawStderr`, abort footer for
  rust-panic / wasm-trap — that don't 1:1 map; needs design pass for
  whether to expand `Error` shape or layer an adapter.)
- [x] ~~Tests in `assets/tests/ErrorTest.elm` pin: `Error.decode`
  round-trip for every variant; severity vocabulary
  (case-insensitive); **unknown severity FAILS** (axiom enforcement);
  identity preservation via `Error.key`; cursor-anchor decode +
  `clickAnchorDecoder` shape vs MouseEvent.~~

### Slice 2 — the Elm sweep (Phase O0a in OBSERVABILITY.md)

- [~] Audit `assets/src/**/*.elm` for `Err _ ->`, `Result.withDefault`,
  `Maybe.withDefault` on Maybes that carry failure. Each site
  constructs a typed `Error` with its surface/region context and
  routes through the Error pipeline.
  **Partial 2026-06-10**: 7 sites in `Main.elm` migrated —
  `BuildInfoReceived`, `BootDataReceived`, `GameMetaReceived`,
  `SpectatorStateReceived`, `UctPreviewReceived`, and both
  `GameStateReceived` subfield-decode swallows. Each constructs a
  typed `Error.Error` via the new `pushDecodeError` helper with the
  appropriate surface tag. `viewSurfaceWithErrors` wrapping is wired
  in `Main.view` for deckbuilder / spectator-bar / prompt / game-meta
  / game-screen / build-footer so each surface anchors its own
  errors locally per the visual contract (`position: relative` host,
  errors filtered by `context.surface`).
  Also shipped 2026-06-11: `errorIn` port (typed JS→Elm Error
  envelope), `Browser.Events.onClick` cursor capture into
  `lastClickAnchor`, `ErrorReceived` Msg that fills anchor from
  captured cursor when the sender didn't supply one. Errors that
  fail to decode on the pipeline ITSELF route through the meta
  error-pipeline surface (no irony-fatal silent drop).
- [ ] Add an Elm-test that fails on new occurrences of `Err _ ->`
  in `assets/src/` (regex-based source scan inside elm-test) so the
  axiom is enforced in CI. (Held — CI integration design needed;
  pre-commit hook is the simpler alternative shape.)

### Slice 3 — the JS sweep (Phase O0b)

- [~] Audit `assets/*.{js,html}` for silent `catch` blocks,
  `.catch(() => {})`, `await` without error handling, `postMessage`
  boundaries that drop errors. Each catches into a typed `Error`
  envelope that flows through the `errorIn` port and lands in the
  same render path.
  **Partial 2026-06-11**: foundation + 7 sites migrated.
  Foundation: `tsotPushError` helper in `js-bridge.js` constructs
  the typed envelope and dispatches through `app.ports.errorIn`;
  `tsotErrorAppRef` stash makes it reachable from module-scope.
  Sites migrated:
  - `tsotShowBridgeFailure` (every workerCmd dispatcher throw +
    every idbReq dispatcher throw routes through it)
  - `withInlineError` (every click action wrapped by it — Save,
    Download, LoadFromFile, TestPanic, StartGame, StartSpectate)
    captures cursor anchor from `MouseEvent.clientX/Y` and pushes
    typed Error with that anchor so the overlay lands AT the click
  - `play.html:325` FFI envelope JSON.parse failure (raw payload
    sampled into the Error)
  - `play.html:607,626` `dbAppendDecision(...).catch` IDB write
    failures (warn-level, non-fatal but visible)
  - `play.html:722` preview-UCT non-FFI render failure
  - `play.html:1048` spectate failure
  - `play.html:1227` engine-start failure
  - `play.html:1268` wasm-worker-spawn failure (panic-level)
  - `play.html:1289` deckbuilder-bootstrap failure
  - `play.html:192` SharedArrayBuffer init failure (warn-level)
  Remaining defense-in-depth catches (console safety nets, DOM
  injection fallbacks) intentionally NOT routed — they're the floor
  that fires when the pipeline itself is broken.
- [~] The existing `js-bridge.js` panic-envelope shim becomes the
  prototype: every JS-side catch produces the same envelope shape,
  not just panics. **Foundation shipped 2026-06-11** via
  `tsotPushError` — single envelope shape, the same Error.elm
  decoder accepts every JS-side push. Only the Test Panic path is
  user-verified end-to-end (the canary). The 7 newly-migrated
  catches in `play.html` (FFI parse / IDB append / preview /
  spectate / engine-start / wasm-worker-spawn / deckbuilder /
  SharedArrayBuffer) build clean but each path needs its own
  verification under a real triggering failure.

### Slice 4 — the Rust + FFI side

- [~] Define `tsot_engine::Error` mirroring the Elm shape. Serde
  serialization byte-compatible with the Elm decoder.
  **Shipped 2026-06-11**: `src/error.rs` with `Error`, `Severity`,
  `Context`, `Anchor`. 6 unit tests pin the wire shape: round-trip,
  lowercase severity strings, optional-field omission, monotonic id
  counter, push/drain/reset semantics. Not yet end-to-end verified
  by user click but the Rust→JS direction is exercised by tests.
  **Extracted 2026-06-18**: type definitions moved to
  `crates/sacred-error/src/lib.rs`; TSOT (`src/error.rs`) and roam
  (`roam/src/error.rs`) both consume them via path dep. The two
  parallel copies that previously drifted (roam's mirrored TSOT's
  but with `at: String::new()` placeholder + `"err-roam-"` id
  prefix) are now thin per-crate bus modules over a shared type.
  Wire-shape tests live in `sacred-error` (4 tests including
  axiom-enforcement: unknown severity label FAILS decode); each
  consumer keeps 3 bus-behavior tests for its id-prefix +
  thread-local. Any future field added to `Error` lands once in
  `sacred-error/src/lib.rs` instead of twice.
- [~] `wasm_ffi.rs` envelope grows an `errors: Vec<Error>` field
  drained from a thread-local buffer (sibling of the `trace` bus).
  **Shipped 2026-06-11**: every FFI envelope path now drains the
  error buffer — `tsot_start_game`, `tsot_apply_action`,
  `tsot_save_game`, `tsot_load_game`, `tsot_run_auto_game`,
  `err_envelope`, plus the new wrap-result paths from OBS O5b.
  `err_envelope` itself emits a typed Error before draining, so the
  Rust-side string error round-trips as a typed value too.
- [~] Every Rust `eprintln!` site that wants to surface to the
  developer pushes an `Error` to the buffer instead of stderr (stderr
  stays as a CLI-only fallback gated behind `cfg(not(target_family =
  "wasm"))`).
  **Partial 2026-06-11**: 3 `lua_api.rs` handler-failure sites
  migrated (`fire_self_only`, `fire_activated`, `fire_with_partner`)
  — they now `error::emit_region(...)` BEFORE the existing
  `eprintln!` (kept as native-CLI fallback). `game.rs:53`
  `bump_timeout_and_maybe_halt` is intentionally NOT migrated —
  reached only by native-EA timeout paths, `process::exit` follows.
  `cli_*.rs` `eprintln!` sites are CLI-binary-only surfaces;
  exempt. The `game.print()` Lua debug print stays as `eprintln` —
  it's intentional dev-tool debug output, not a failure.
- [~] `handler.call` errors (Lua-yield bug area) push `Error`s too;
  the existing `eprintln!("[lua] {event} handler for {card_id} failed: {e}")`
  pattern is the obvious first conversion. **Done 2026-06-11** via
  the previous bullet — all 3 fire_* call sites push typed Errors.

### Slice 5 — contextual surfaces (Phase O0c)

- [~] Each Elm surface that can fail declares its context up front
  (deckbuilder dropdown = `Context { surface = "deckbuilder", region = Just "preset-dropdown" }`).
  Failures attached to that context render inline at the surface,
  not just in LOG.
  **Partial 2026-06-11**: cursor-anchored rendering is shipped —
  click-driven failures spawn a classic-OS window with severity-
  tinted titlebar + × close button + drag-by-titlebar AT the cursor
  position. Viewport-aware corner flip (classic context-menu
  behavior: cursor sits on whichever box corner has room) so the
  box opens INTO the viewport. Surface-anchored fallback (port
  decode failures with no cursor) renders inside the surface's
  positioned container via `viewSurfaceWithErrors`. What's left:
  the named-region inline canary for specific decoder failures
  (next bullet).
- [~] The deckbuilder dropdown shows "decode failed: 1 preset rejected
  — <field>" inline as the canary case the original audit session
  found.
  **Shipped 2026-06-11**: `tsot_list_preset_decks_impl` now
  validates every `card_id` in every preset against the playable
  pool and emits a typed `Error` (severity Warn, surface
  `"deckbuilder"`, region `"preset-dropdown"`) for each miss. The
  envelope `errors[]` field carries them to the JS dispatcher
  which forwards through `tsotPushError`, where they render
  inside the `deckbuilder` surface container via the existing
  `viewSurfaceWithErrors` wrapping. The original preset-mystery
  failure mode now surfaces. (Not yet user-verified end-to-end;
  the validation is a no-op when every preset is well-formed,
  which is the current corpus state.)
- [ ] The prompt bar shows handler errors inline when a Lua handler
  emits an Error mid-cast. (Requires Slice 4.)

### Slice 6 — invariant + lifecycle parallel with CARD.md

By analogy to CARD.md Slice 5 (state persistence + bijectivity
invariant): an Error has stable identity for its entire lifetime in
the session. Once emitted, the same Error.id maps to the same DOM
node across re-renders. Dismissal is a state transition on the
existing element, not a destroy + reconstruct. Persists across page
reloads via `localStorage` so a failure that happened mid-session
survives the developer pressing F5.

## Relation to other docs

- `CARD.md` — defines the rendering axiom for the game's primary
  entity. `ERROR.md` does the same for failures. Same architectural
  treatment.
- `OBSERVABILITY.md` — covers the *engine's* narration of internal
  events (the trace bus). The Error primitive sits one layer up: it
  carries failures across system boundaries, where the trace bus
  carries successes (or attempted successes). OBSERVABILITY's
  Phase 0 (Elm + JS silent-drop sweep) maps directly to Slices 2–3
  of this document.
- `CLAUDE.md` — *"errors are sacred — first-class citizens, never
  collapsed, dropped, swallowed or suppressed. If an error is not
  visible or surfaced, drop everything you do, and make sure we see
  the error FIRST before continuing with anything else."* This
  document operationalizes that policy.
- `LIMITATIONS.md` — known gaps. Once `ERROR.md` ships through Slice
  3, this doc's section on "errors that don't surface" can be
  trimmed because the axiom forbids them by construction.
