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
of interaction.*

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

### Engine internals (medium)

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

Only the active slice. Earlier slices' state lives in the Inventory
above; shipped work lives in git history.

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
  carries successes (or attempted successes).
- `CLAUDE.md` — operationalizes the *"errors are sacred"* policy
  via the axiom block at the top.
