# Cardless Sleeve & Sleeve-as-Atom — plan of record

> The sleeve is the atomic unit in every zone; a card is optional content
> inside it. 0 cards = cardless sleeve (Z.8), 1 = a normal card, 2+ = a
> same-sleeve fusion (Z.7). This unifies C.16, V.7b, and Z.7 under one
> ontology.

## Status

- **Slice 4 — DONE.** `CardInstance → Sleeve` rename.
- **Slice 5 — DONE (approved).** `Sleeve.content: Option<Card>` + `card()`
  blank-fallback accessor + `card_mut()` + `is_cardless()`. Emptiness is
  representable; nothing creates a cardless sleeve yet. 492 lib + all
  integration green.
- **Slice 6 — APPROVED, next.** Z.8b free draw.
- **Slice 7 — APPROVED.** Z.8c cost payment.
- **Slice 8.** Deck-as-units, search-for-cardless, Window Cleaner.
- **Spec.** Write Z.8 + S.4 amendment into RULES.md alongside behaviour.

## Z.8 — CARDLESS SLEEVE (agreed spec, not yet in RULES.md)

A sleeve-unit containing no card. No color, no symbol, no printed rules;
cannot be cast.
- **Z.8a** Untargetable — no card inside → no front-visible face → nothing
  can target it (C.4).
- **Z.8b** Draw — a cardless sleeve on top of DECK does not satisfy "draw a
  card": it is collected into HAND for free and the draw continues,
  cascading through consecutive cardless sleeves until one card-bearing
  unit is drawn.
- **Z.8c** Cost payment — counts as a generic body for HAND, GRAVEYARD, and
  (while attached) ATTACHED cost sources. Never counts for MILL. Never
  satisfies the color/symbol identity requirement of any cost (P.7a) — it
  fills a slot, not an identity.
- **Z.8d** Attachment — may be attached (Z.6) to a card by an effect
  (Window Cleaner); while attached it can be spent to pay an ATTACHED cost.
- **Z.8e** Not fillable (current) — a card cannot be moved into a cardless
  sleeve; consumable blank. *[Deferred: the "worn" concept, own branch.]*
- **S.4 amended** — "a deck contains 50 cards" → **50 sleeve-units** (a
  cardless sleeve is a legal unit; empties occupy a unit).

## MUST-DO (engineering, inside this branch)

1. **Cardless-sleeve creation primitive + journaling.** Nothing builds one
   yet. Gate for slice 6. Every new mutation (create, draw-skip collect,
   exile-to-pay) must journal, per the rollback invariant.
2. **`card_mut()` invariant audit.** `card_mut()` panics on a cardless
   sleeve by design (no card to mutate). Once cardless sleeves exist,
   confirm no write path can reach one.
3. **Sim/AI awareness.** Cardless sleeves will sit in decks and hands.
   Not-castable is already handled (blank kind = Unspecified, AI filters
   it). Still needed: the free-draw during AI games, and treating a
   cardless sleeve as valid payment fuel in cost selection.
4. **RULES.md Z.8 + S.4** written alongside the behaviour (rules+code move
   together).
5. **Deckbuilding data format** — how a cardless sleeve is expressed in a
   decklist / EA genome so it can exist in real games (needed for slice 8).

## REQUIRES USER INPUT (design)

- **Terminology.** Is "sleeveless" (from Shatter Expectations) the same as
  "cardless"? Is "clear" = transparent-frame (C.13/C.14)? Confirm so the
  card text maps to engine concepts.
- **How cardless sleeves enter a deck** — decklist/genome representation,
  and legality: S.4 as 50 units, any cap on how many empties, minimum real
  cards.
- **C.14 for cardless sleeves.** A cardless sleeve is frameless (no card =
  no frame). Can it attach to any host, or only transparent hosts? Window
  Cleaner has transparent slots and attaches cardless sleeves — is there a
  required transparent-compat relationship?
- **AI/EA valuation** — should the EA be allowed to draft cardless sleeves?
  (Affects genome format and fitness.)

## DEFERRABLE

- **Elm UI** — out of scope for this branch (rendering cardless sleeves,
  the free-draw animation, attach visuals). TODO noted in ELM_PLAN.md.
- **"Worn" + fillable sleeves (Z.8e)** — own branch (putting cards in/out
  of sleeves).
- **Window Cleaner on-tap trigger** — needs an OnTapped event (verify it
  exists). Slice 8 or later.
- **Shatter Expectations** — capstone card; needs the counter-unless-pay
  mechanic (below). After the core cardless slices.
- **Enforcing S.4 legality** — only if/when a deck-legality check exists.

## Card backlog

### Window Cleaner (human, cyan)
- Transparent slots T, TR, RU, RC, C. Cost `2 attach`. 2/3 reach.
- ETB: search library for 2 cardless sleeves, attach them to this card.
- On becoming tapped: *may* move an attached cardless sleeve to GY and draw
  a card. **No inherent tap ability** — relies on being tapped for an
  attack or by another effect.
- Loop: the 2 attached cardless sleeves are attach-cost fuel for the next
  Window Cleaner.
- New engine needs: OnTapped trigger (verify), search-for-cardless.

### Shatter Expectations (instant, colourless)
- Entire top and bottom rows: transparent slots.
- Cost: **X graveyard**, where X = the number of **sleeveless and clear**
  cards exiled to pay for it. *(Open: confirm X is defined by the count of
  cardless + transparent cards exiled.)*
- Effect: **Counter target spell, unless its controller exiles X from their
  HAND, X from GY, X from BOARD, and X from DECK.** (Opponent may pay 4X to
  save the spell.)
- Flavour: "he paid it!?"
- New engine needs (all deferred):
  - **Counter-with-alternative-cost** — a counter the *targeted* player can
    negate by paying. Today `game.counter` is unconditional; this needs an
    opponent-side may-cost prompt through the choice/oracle system.
  - **Variable X paid by exiling cardless + clear** — X = payment count.
  - **Multi-zone exile** (X from each of hand/gy/board/deck) as a payment
    shape.
- Open questions: exact X definition; is the escape cost a "may" the
  opponent chooses at counter-resolution; do cardless/clear give a discount
  or just define X.
