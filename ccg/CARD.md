# tsot — Card Axiom

## The axiom

**A Card is exactly one DOM element for its entire lifetime.**

Cards are reparented, never cloned. Face state, hover state, focus, scroll
position, in-flight CSS transitions, and any rendered subtree state survive
every transition between manifestations (hand → board → attached → graveyard
→ exile → back to board on rewind, etc.).

Violation = the card "teleports" instead of moving. The DOM destroyed it and
constructed a fresh one. From the user's point of view, the previous card
ceased to exist and a new one appeared in the new zone. That's a different
card, not the same card relocating.

## Why the axiom is necessary

- **Identity continuity.** A creature on the board is the same object the
  player put there from their hand; the engine knows this (stable iid), the
  UI should too. A re-rendered fresh element breaks the link.
- **Animation is impossible without identity.** "Cards move smoothly" only
  has meaning when there's a same-thing to animate from A to B. Destroy +
  construct = teleport, no matter how many keyframes you wrap around it.
- **Hover / focus / scroll survive renders.** If hover state lives on a
  specific DOM node and that node is gone after the next snapshot,
  every redraw wipes the user's hover. The Phase 4 patience-stack popover
  was an example: hovering a peek created a separate Front node. Two
  different cards in the DOM for one card in the game.
- **Engine snapshots are frequent.** Every state mutation re-runs the view.
  At that rate, cloning everything per render is wasteful AND wrong.

## What the axiom forbids

1. **Cloning a card to render it twice.** No "back peek + front popover"
   pairs. No "compact list row + full card detail" pairs. The card is one
   element; views are CSS state on it.
2. **Destroying a card on zone change.** A card moving from hand to board
   is not "remove from hand container, append to board container" — that's
   destroy + construct in vDOM terms. It's "the same node, now positioned
   in the board's region."
3. **Degenerating a card into a tooltip.** A tooltip representing card data
   is not a card. The card is already there; show its face by flipping it.
4. **State decay between renders.** Hover, focus, scroll, animation —
   nothing the browser tracks per-node should be reset by an engine event
   the user didn't initiate.

## What the axiom requires

1. **One DOM element per iid, always.** A flat top-level container holds
   every card as a sibling, identified by `data-iid` (or as `Html.Keyed`
   key). Zone membership is a CSS-positioned property of the element —
   `data-zone="your-board"` translates to `transform: translate(...)` or
   `grid-area: ...` placing the card in that zone's visual region.
2. **Face state via CSS class.** The same element renders the face-up DOM
   (head + meta + abilities); a `.face-down` class hides the face-only
   children so only symbols show. `:hover` (or any other state trigger)
   removes the hide. The card is rendered ONCE in either case.
3. **Reparenting via CSS, never via vDOM.** Zone transitions are state
   changes on the element (a class flip, a transform target). The element
   never changes parents in the DOM. CSS transitions on the positioning
   property give the visible "move".
4. **Stable identity across game events.** The iid is the only identity
   the UI cares about. As long as the iid is the same, the DOM node is the
   same.

## Current state (2026-06-09)

Violated in ~~three~~ one measurable way.

### ~~Violation 1 — Phase 4 popover doubles the DOM per attached card~~

`Card.viewAttachedRow` renders a `.attached-slot` (clipped Back) AND a
`.attached-popover` (full Front) for every attached card. Two `.card`
divs per iid. Hover swaps which one is visible. The popover and the peek
are not the same element.

### ~~Violation 2 — Front and Back are separate render paths~~

`Card.view` dispatches on `RenderMode` to `viewFront` vs `viewBack`,
emitting structurally different DOM. A card going from face-up to
face-down (deck-top transition, attached transition) produces a
completely different DOM subtree. Not one element with two states; two
different elements at different times.

### Violation 3 — every zone is its own DOM parent

The in-game render assigns cards to `#your-hand-cards`, `#your-board-cards`,
`#your-graveyard-cards`, etc. — five different parents per player. A card
moving from hand to board is removed from one parent and added to another.
Elm's virtual DOM has no way to preserve identity across parents; the result
is destroy + construct = teleport.

`Html.Keyed` doesn't fix this. Keyed preserves identity WITHIN one parent
when children reorder. Cross-parent moves still destroy + construct.

## Roadmap to compliance

Five slices, smallest to largest.

### ~~Slice 1 — single render path with `.face-down` state class~~

- Drop `RenderMode` from `Card.elm`. Drop `viewFront` / `viewBack`. One
  `view` function emits the full face-up DOM unconditionally.
- Add `faceDown : Bool` to `Card.Config`. Emit class `face-down` on the
  card div when set.
- CSS rules: `.card.face-down .head`, `.card.face-down .abilities`,
  `.card.face-down .meta-line > :not(.symbol)` → `display: none`.
  `:hover` reverts (inverse rules with `display: revert`).
- Deck-top renders use `Card.view { defaultConfig | faceDown = True } card`.
- Phase 4 attached strip renders each attached as ONE card with
  `faceDown = True`, wrapped in a `.attached-slot` that clips visually.
  No popover. Hover on the slot triggers the card's `:hover`, which
  reveals the face content. Same element flips.

Closes Violation 1 + Violation 2.

### Slice 2 — `Html.Keyed` at every zone container

- All zone child lists use `Html.Keyed.node "div"` with `iid` as the key.
- Within a zone, reorderings preserve identity. A card swapping board
  position with its neighbor doesn't re-render.
- Tests pin keyed-DOM identity by introspecting the Elm output for a
  before/after snapshot.

Partial progress on Violation 3 — intra-zone identity preserved.

### Slice 3 — card pool architecture (the big one)

- Introduce `#card-pool` — a single top-level container that holds EVERY
  card iid in the current state as a direct child. No nested zone
  containers as DOM parents.
- Each card has `data-zone` + `data-zone-index` attributes (or whatever
  the engine snapshot makes natural). Zones are CSS regions, not parents:
  `[data-zone="your-board"]` → `transform: translate(boardX, boardY)`.
  Position within the zone derived from `data-zone-index`.
- The visible "your hand", "your board", etc. boxes in the layout become
  pure CSS regions / overlays; they contain no card DOM. They're labels
  and borders, not parents.
- The Card view function in Elm becomes responsible only for the card's
  internal DOM. Placement is the snapshot decoder's job — it emits the
  full pool with positioning metadata; CSS does the rest.

Closes Violation 3 fully. Cards never change DOM parents.

### Slice 4 — animated zone transitions

- Once Slice 3 lands, "card moved from hand to board" is a `data-zone`
  attribute change → triggers a CSS transition on `transform`. Smooth
  motion is automatic.
- FLIP technique unnecessary because the DOM node never moved; only its
  CSS computed position did.
- Tune per-transition timing (draw fast, attach slow, etc.) by varying
  CSS transition duration based on the source/destination zone pair.

### Slice 5 — state persistence audit

- Confirm hover state survives engine snapshots. (Should be free after
  Slice 3 — same DOM node, browser-tracked hover doesn't reset.)
- Confirm focus survives — keyboard focus on an action button inside a
  card should persist when the snapshot updates.
- Confirm in-flight CSS transitions don't restart on snapshot.
- Add a runtime invariant check: a debug build asserts that the
  `iid → DOM node` mapping is bijective each render. Violation throws
  `INVARIANT VIOLATION` in the dev tool.

## Relation to other docs

- `RULES.md` defines what a card IS in the game (C.1–C.17b — colors,
  symbols, states, identity). This document defines how a card IS
  REPRESENTED in the UI. RULES is the game; CARD is the rendering
  contract for the game's primary entity.
- `SLOTS.md` defines the back-of-card 5×3 grid for symbol + hole
  placement. Compatible with the axiom — it describes the internal DOM
  layout of a single card, not multiple cards.
- `LIMITATIONS.md` will gain a `## card-axiom` entry listing the
  current violations (the three above) and pointing here for the
  roadmap. Slices close one by one; mark them through in the roadmap
  above (`~~Slice N~~`) as they ship.
