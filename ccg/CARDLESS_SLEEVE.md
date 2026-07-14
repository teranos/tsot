# Cardless Sleeve & Sleeve-as-Atom — plan of record

> The sleeve is the atomic unit in every zone; a card is optional content
> inside it. 0 cards = cardless sleeve (Z.8), 1 = a normal card, 2+ = a
> same-sleeve fusion (Z.7). Rules are canonical in RULES.md (Z.8, S.4);
> this doc is the roadmap, not the spec.

## Status

- **Slice 4 — DONE.** `CardInstance → Sleeve` rename.
- **Slice 5 — DONE.** `Sleeve.content: Option<Card>` + `card()` blank-
  fallback accessor + `card_mut()` + `is_cardless()`. Emptiness is
  representable; nothing creates a cardless sleeve yet.
- **Slice 6 — DONE.** Z.8b free draw (`draw_one` primitive).
- **Slice 7 — DONE.** Z.8c cost payment + Z.8e visibility. Attach / wildcard-
  hand / can't-anchor were already free (blank card = empty identity); code
  added for the anchor+cardless-body HAND case and MILL exclusion. All
  changes `is_cardless`-guarded → no-ops for cardless-free decks.
- **Spec — DONE.** Z.8 + S.4 in RULES.md.
- **Slice 8 — cardless sleeves become real & engine/AI-correct.** Medium
  (≈ slice 7). Three subunits, each green on its own:
  - **8.1 Representation + serialization — DONE.** `DeckUnit {Card, Cardless}`
    + `GameState::from_units`; `new(Vec<Card>)` unchanged (wraps as Card
    units). `rebind_handlers` skips cardless; `ReplayFile` uses the
    `CARDLESS_SLEEVE_ID` sentinel so cardless deck units round-trip. Tests in
    `tests/cardless_sleeve.rs` (from_units placement, save/load, replay
    rebuild).
  - **8.2 AI affordability — DONE (picker/resolver agree; no loops).**
    Auditing the model showed only MILL could disagree: the resolver counts
    card-bearing sleeves (slice 7) but `can_pay_instant_cost` counted total
    deck → fixed to count `!is_cardless` cards. The rest already agree:
    wildcard-hand and attach include cardless in `eligible_hand_payments` /
    `attached_have`; GY is guarded by the P.12a anchor check. Tests in
    `game/cardless_sleeve_tests.rs` per cost source. **Deferred optimization
    (not a loop):** the AI does not yet exploit cardless as bodies for an
    *identity* HAND cost — it stays conservative (no anchor → refuse), which
    matches the resolver, so it never loops, it just misses some castable
    plays. Wiring that needs anchor-first hand selection; revisit if a card
    makes it matter.
  - **8.3 End-to-end acceptance — DONE.** `sim/run.rs` tests build a deck
    from `DeckUnit`s with cardless sleeves and run a full Heuristic-vs-
    Heuristic game: it completes with a winner, the Z.8b free draw pulls a
    cardless sleeve off the deck, the full-game replay journal rolls back to
    the exact initial state (cardless sleeves included), and two identical
    runs are byte-identical (determinism). No integration fixes were needed —
    the whole 4→8 arc holds in a real game.
- **Slice 9 — the cards + the end-to-end test deck.**
  - `search library for cardless sleeves` primitive.
  - Window Cleaner (see backlog). OnTapped trigger — verify/add.
  - Supporting cards: clears (transparent) + an azure symbol.
  - Acceptance (user's target): a deck of Window Cleaners, clears, an azure
    symbol, and cardless sleeves plays a full game.
- **Slice 10 (later) — Shatter Expectations** (see backlog). Needs
  counter-with-alternative-cost + composition-derived X + multi-zone exile.

## Watch-outs

- **`card_mut()` invariant.** Panics on a cardless sleeve by design (no card
  to mutate). Once cardless sleeves exist, confirm no write path reaches one.
- **Visibility opacity.** `effective_top_of_deck_symbols` treats every
  cardless sleeve as transparent — correct while all are clear; gate on
  sleeve opacity once opaque colored sleeves are modeled (an opaque one
  blocks, Z.8e).

## Open questions (user input)

- Is "clear" = transparent-frame (C.13/C.14)? (Very likely yes.)
- **Deckbuilding format** — how a cardless sleeve is expressed in a decklist
  / EA genome; slice-8 uses a hand-authored fixture, EA-genome TBD.
- **C.14 for cardless sleeves** — a cardless sleeve is frameless; can it
  attach to any host, or only transparent ones?
- **EA valuation** — should the EA draft cardless sleeves? (Affects genome.)

## Deferred

- **Elm UI** — out of scope this branch (rendering, free-draw animation,
  attach visuals). TODO in ELM_PLAN.md.
- **Worn + fillable sleeves** — own branch (putting cards in/out of sleeves).
- **Opaque / colored sleeves** — the sleeve-color/opacity model (colored
  transparent + opaque colored-back sleeves that carry color and can satisfy
  color costs). Beyond cardless; touches Z.8e visibility.
- **Enforcing S.4 legality** — only if/when a deck-legality check exists.

## Card backlog

### Window Cleaner (human, cyan)
- Transparent slots T, TR, RU, RC, C. Cost `2 attach`. 2/3 reach.
- ETB: search library for 2 cardless sleeves, attach them to this card.
  Window Cleaner only ever gives **transparent** (clear) cardless sleeves.
- On becoming tapped: *may* move an attached cardless sleeve to GY and draw
  a card. **No inherent tap ability** — tapped by an attack or another effect.
- Loop: the 2 attached cardless sleeves are attach-cost fuel for the next
  Window Cleaner.

### Shatter Expectations (instant, colourless)
- Entire top and bottom rows: transparent slots.
- Cost: **X graveyard** — you exile cards to pay.
- **X is derived from the payment composition:** `X = (#clear + #cardless
  sleeves exiled) − (#non-clear-non-cardless cards exiled)`. Each clear or
  empty adds 1; each ordinary card subtracts 1. Pure clear/empty maximises X.
- Effect: **Counter target spell, unless its controller exiles X from HAND,
  X from GY, X from BOARD, and X from DECK** (4X total). The controller
  **chooses** whether to pay.
- Flavour: "he paid it!?"
- New engine needs: counter-with-alternative-cost (opponent-side may-pay
  prompt); composition-derived X; multi-zone exile.
- Open edge: X ≤ 0 (all-ordinary or net-negative payment) — floor at 0? legal?
