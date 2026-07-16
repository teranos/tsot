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


## Current inventory

Per-project migration state lives with each consumer of the axiom:

- ccg — see [`ccg/ERROR_INVENTORY.md`](./ccg/ERROR_INVENTORY.md).

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
   side — serde + Elm decoders agree byte-for-byte. The type lives
   in `crates/sacred-error/src/lib.rs` and is consumed by both `tsot`
   (via path dep, see `Cargo.toml`) and `roam`.
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

See "Doc / axiom open items" above for the full design slice. The
short version: Error has stable identity for its entire lifetime in
the session, the same Error.id maps to the same DOM node across
re-renders, and the errors list persists across page reloads via
`localStorage` so a failure that happened mid-session survives the
developer pressing F5.

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
- `crates/sacred-error/src/lib.rs` — the typed Error wire shape +
  axiom-enforcement tests (severity must be one of four labels;
  unknown FAILS decode). The Rust side of the contract.
- `.github/workflows/sacred-error-check.yml` — the CI grep that
  fails the build when new silent-drop patterns appear.
- `.github/sacred-error-baseline/` — the counted snapshot of
  acceptable existing matches per pattern. Updating a baseline is
  reviewable in the diff.
