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
  - **9.1 — DONE.** `attach_cardless_from_deck` (Rust + Lua): search deck
    for cardless sleeves, attach n.
  - **9.2 — DONE (attack tap only).** `EventName::OnTapped`, fired from
    combat declare_attacker (gated !vigilant). External taps deferred
    (firing inside a Lua borrow needs a deferred-event queue). No-op for
    cards with no `on_tapped` handler.
  - **9.3 — cards.** Window Cleaner (ETB attach 2 cardless; `on_tapped` →
    may move an attached cardless to GY + draw), clears (transparent), an
    azure symbol.
  - **9.4 — test deck end-to-end** (user's target): a deck of Window
    Cleaners, clears, an azure symbol, and cardless sleeves plays a full
    game.
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
- **Deferred-event queue** (slice 11 candidate) — a queue for events that
  can't fire synchronously because they'd re-enter a Lua borrow. Unblocks:
  OnTapped on *external* taps (`game.set_tapped`, not just attack); the
  delayed-trigger registry (LIMITATIONS); and Shatter's counter-may prompt.
  The single most-enabling piece of remaining engine work.

## Queued tasks

- **Empty sleeve in every starter deck** (user request). Real change, not a
  one-liner: the runtime deck path (`to_deck → Vec<Card> → shuffle_deck →
  GameState::new`) is `Vec<Card>` throughout; adding a cardless unit means
  threading `DeckUnit` through it (the sentinel → `Cardless`, shuffle over
  units, build via `from_units`) and adding the sentinel to the two starter
  id lists. Same shape as 8.1 but for the live start pipeline.
- **EA drafts cardless sleeves?** — currently NO (the EA pool is
  `Vec<Card>`; genome/deckbuilder never emit cardless). Open design call.

## Known issues

- **`diversity_alpha_widens_final_population_diversity` — flaky under load,
  not a real bug.** The sim's wall-clock watchdogs (`run.rs`,
  `TSOT_GAME_TIMEOUT_SECS`, `watchdog_pattern_b_walltime`) assign a winner by
  *elapsed time*, so under heavy parallel CPU load some games trip the
  timeout → outcomes shift → this EA-diversity comparison flips. Deterministic
  in isolation; flaky under `--tests`. Reproduce by lowering
  `TSOT_GAME_TIMEOUT_SECS` or running under load. Not reproducible as a single
  game (it's an aggregate metric, and the non-determinism is wall-clock, not
  seed). Real fix would be a count-based (not wall-clock) watchdog.

## Card backlog

### Window Cleaner (human, cyan)
- Transparent slots T, TR, RU, RC, C. Cost `2 attach`. 2/3 reach.
- ETB: search library for 2 cardless sleeves, attach them to this card.
  Window Cleaner only ever gives **transparent** (clear) cardless sleeves.
- On becoming tapped: *may* move an attached cardless sleeve to GY and draw
  a card. **No inherent tap ability** — tapped by an attack or another effect.
- Loop: the 2 attached cardless sleeves are attach-cost fuel for the next
  Window Cleaner.

### Angry Glassblower (red creature)
- 3/4. Cost: 2 HAND + 1 GY.
- On attack: *may* attach an empty sleeve to it and draw a card.
- On dealing damage to an opponent: *may* exile an attached card from it; if
  it was an empty sleeve, draw a card and discard a card.
- Uses existing events (OnAttack, OnDealtDamageToPlayer) — no OnTapped
  needed. Writable now given the search/attach + exile-attached primitives.
- Open: does "attach an empty sleeve" search the library (like Window
  Cleaner) or create one from nothing? (No create-from-nothing primitive
  exists yet.)

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
