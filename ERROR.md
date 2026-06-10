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
- [ ] Add an Elm-test that fails on new occurrences of `Err _ ->`
  in `assets/src/` (regex-based source scan inside elm-test) so the
  axiom is enforced in CI.

### Slice 3 — the JS sweep (Phase O0b)

- [ ] Audit `assets/*.{js,html}` for silent `catch` blocks, `.catch(() => {})`,
  `await` without error handling, `postMessage` boundaries that drop
  errors. Each catches into a typed `Error` envelope that flows through
  the `errorOut` port (JS → Elm) and lands in the same render path.
- [ ] The existing `js-bridge.js` panic-envelope shim becomes the
  prototype: every JS-side catch produces the same envelope shape,
  not just panics.

### Slice 4 — the Rust + FFI side

- [ ] Define `tsot_engine::Error` mirroring the Elm shape. Serde
  serialization byte-compatible with the Elm decoder.
- [ ] `wasm_ffi.rs` envelope grows an `errors: Vec<Error>` field
  drained from a thread-local buffer (sibling of the `trace` bus).
- [ ] Every Rust `eprintln!` site that wants to surface to the
  developer pushes an `Error` to the buffer instead of stderr (stderr
  stays as a CLI-only fallback gated behind `cfg(not(target_family =
  "wasm"))`).
- [ ] `handler.call` errors (Lua-yield bug area) push `Error`s too;
  the existing `eprintln!("[lua] {event} handler for {card_id} failed: {e}")`
  pattern is the obvious first conversion.

### Slice 5 — contextual surfaces (Phase O0c)

- [ ] Each Elm surface that can fail declares its context up front
  (deckbuilder dropdown = `Context { surface = "deckbuilder", region = Just "preset-dropdown" }`).
  Failures attached to that context render inline at the surface,
  not just in LOG.
- [ ] The deckbuilder dropdown shows "decode failed: 1 preset rejected
  — <field>" inline as the canary case the original audit session
  found.
- [ ] The prompt bar shows handler errors inline when a Lua handler
  emits an Error mid-cast.

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
