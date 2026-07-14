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
- **Slice 8 — cardless sleeves become real & engine/AI-correct.**
  - Creation primitive (build a cardless sleeve unit) + journaling.
  - Deck-as-units: a deck can contain cardless-sleeve units (S.4).
  - AI-side wiring (deferred from slice 7): cardless in
    `eligible_hand_payments` + affordability (`identity_matching_hand_count`,
    `can_pay_instant_cost` mill branch) so picker and resolver agree. Not
    exercised today — nothing puts a cardless sleeve into an AI game yet.
  - Acceptance: a hand-authored test deck with cardless sleeves runs a full
    sim game; determinism + full-game rollback hold.
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
