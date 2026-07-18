# Cardless Sleeve & Sleeve-as-Atom — plan of record

> The sleeve-unit is the atom in every zone; a card and a sleeve are each
> optional occupants of it. 0 cards = cardless sleeve (Z.8), 1 = a normal
> card, 2+ = a same-sleeve fusion (Z.7); a card with its sleeve removed =
> a sleeveless card (Z.8, the mirror of the cardless sleeve). Rules are
> canonical in RULES.md (Z.8, S.4); this doc is the roadmap, not the spec.

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
    `game/cardless_sleeve_tests.rs` per cost source. **Former deferral —
    NOW DONE (anchor-first hand selection):** the AI now uses cardless
    sleeves as non-anchor bodies for an *identity* HAND cost, mirroring the
    Clear View GY-substitute path. Affordability counts cardless bodies
    (identity casts only, so no wildcard double-count) on top of a required
    real identity anchor; the resolver fills the shortfall with cardless
    bodies after identity matches + GY substitutes, always leaving ≥1 real
    anchor so play_card's all-cardless gate never fires. Proven loop-free by
    a 2000-game `AiKind::Stress` soak on random cardless decks. Tests in
    `game/cardless_sleeve_tests.rs`
    (`z8_can_pay_identity_hand2_with_one_real_anchor_plus_a_cardless_body`).
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
  - **9.2 — DONE.** `EventName::OnTapped`, fired from combat
    declare_attacker (gated !vigilant). External taps (`game.tap` inside
    a handler) now fire it too, via the slice-11 deferred-event queue.
    No-op for cards with no `on_tapped` handler.
  - **9.3 — DONE.** `cards/window-cleaner.lua` — azure human, `2 attach`,
    2/3 reach, transparent holes T/TR/UR/R/C. ETB attaches 2 cardless
    (via `attach_cardless_from_deck`); `on_tapped` *may* move an attached
    cardless to GY + draw. Added the `game.is_cardless(iid)` Lua primitive
    (cardless-aware cards pick an empty sleeve out of `self.attached`).
    Tests in `game/window_cleaner_tests.rs` (ETB, tap-confirmed,
    tap-declined). Clears (`clear-azure`) and azure symbols already exist.
  - **9.4 — DONE (user's target).** Not a hand-authored fixture: the
    shipped blue starter deck, copied, with a slice of its `clear-blue`
    swapped for the azure cardless stack (4 Window Cleaners, 4
    `clear-azure`, 4 loose cardless sleeves) and its blue ix symbols
    swapped for azure. The real cards in a real deck play a full game to
    a winner with rollback + determinism holding. Tests in `sim/run.rs`
    (`full_game_on_a_window_cleaner_deck_runs_and_rolls_back` +
    `..._is_deterministic`). Slice 9 complete.
- **Slice 10 — DONE. Shatter Expectations** (`cards/shatter-expectations.lua`).
  Colourless instant, top+bottom rows transparent (holes). All three novel
  pieces resolved in `on_play`: composition-derived X (caster exiles chosen
  GY cards; clears + cardless add 1, ordinaries subtract 1), an opponent-side
  may-pay via `game.confirm_for`, and multi-zone exile (X from HAND/GY/BOARD/
  DECK). Non-positive X whiffs (ransom trivially met → spell resolves). Added
  `game.is_clear(iid)` Lua binding. The may-pay turned out to be synchronous
  via `confirm_for` — the slice-11 queue de-risked it but wasn't required.
  Tests in `game/shatter_tests.rs` (opponent pays, opponent declines →
  counter, X≤0 whiff).
- **Slice 12 — sleeve conservation: card ↔ sleeve fully decoupled.**
  - **12.1 Mutation cast sheds its vacated sleeve — DONE.** A mutation sits
    in HAND inside its own sleeve; casting it (P.26) slides the card into
    the host's sleeve (Z.7 fusion) and leaves its own sleeve empty. That
    sleeve is not destroyed — it is minted as a cardless sleeve and
    `add_attached` to the host (Z.6), counted by `AttachedCount` (the card
    doesn't count, only the shed sleeve does). New `JournalEntry::
    MintCardlessSleeve` (forward inserts / inverse removes) so rollback and
    replay drop the shed sleeve. Test in `game/same_sleeve_tests.rs`
    (`z7_mutation_cast_sheds_its_vacated_sleeve_as_an_attached_cardless`).
  - **12.2 Sleeveless state + self-shed — DONE.** `Sleeve.sleeveless: bool`
    (`#[serde(default)]`, backward-compatible) — the mirror of a cardless
    sleeve: a card with no sleeve around it. `shed_own_sleeve(iid)` pops a
    card out of its own sleeve (card stays put, becomes sleeveless) and
    attaches the vacated sleeve to itself as a cardless sleeve — the
    mutation-shed shape, self-targeted. No-op if already sleeveless or if
    the unit is itself cardless. Journaled via `SetSleeveless` + the
    slice-12.1 mint/attach entries. Tests in `game/cardless_sleeve_tests.rs`
    (`z8_a_card_sheds_its_own_sleeve_and_becomes_sleeveless`,
    `z8_shed_own_sleeve_round_trips_through_journal`).
  - **12.3 Death-replacement hook — DONE.** `EventName::OnWouldDie` fires
    self-only on a dying creature BEFORE any Board→GY move (the window
    `OnDie` never gave — it fires *after*). The handler signals via two
    primitives: `game.prevent_death(self)` → survives on the BOARD, engine
    clears its accumulated damage; `game.redirect_death(self, zone)` → moves
    Board→zone quietly instead of GRAVEYARD (no on_die, no OnCreatureDies
    broadcast, no P.8 cascade). No call → normal death. A single chokepoint,
    `GameState::resolve_board_deaths(to_kill, ctx) -> Vec<InstanceId>`, now
    owns the death sequence; the combat death loop and `cleanup_zero_y_
    deaths` both route through it (behaviour-preserving — with no replacement
    it's the same GY-move + on_die + broadcast + cascade as before). Also
    exposed the 12.2 primitive to Lua: `game.shed_own_sleeve`,
    `game.is_sleeveless`. Tests in `game/death_replacement_tests.rs` (shed &
    survive, sleeveless → exile, ordinary-creature baseline, a real
    `confirm_blocks` combat, and a direct `game.damage` burn — all of which
    the elephant survives).
  - **12.3b Direct-damage path reaches the hook — DONE.** `game.damage`
    (`do_damage`) runs inside a live handler's borrow, so it can't fire
    `OnWouldDie` synchronously (re-entrant Lua = RefCell double-borrow). The
    B.8 death sweep is no longer done eagerly in `do_damage`; it is deferred
    to `drain_deferred_events`, which — after the dealing handler unwinds,
    with a Lua ctx — routes lethal creatures through `resolve_board_deaths`.
    So a burn death now reaches the same replacement window as a combat
    death (and, incidentally, now fires `on_die` on burn kills — closing the
    old combat.rs TODO). Re-entrancy guard: `GameState::settling_deaths`
    (transient) makes the nested drains that death-handlers trigger skip the
    scan, so a creature isn't re-killed mid-resolution; chained deaths (a
    death trigger that burns another creature) are caught by the settle
    loop's next pass. Test: `elephant_survives_a_direct_damage_death`.
  - **12.3c Chained combat deaths settle in-combat — DONE.** `confirm_blocks`
    resolves its combat deaths once, under the `settling_deaths` guard, so a
    combat death whose `on_die` burns a bystander used to leave that second
    death standing until the next drain. `drain_deferred_events` now returns
    the deaths it settled, and `confirm_blocks` runs one guard-released drain
    after its combat-death resolution — the chained death resolves within the
    same combat and is folded into `outcome.deaths` (which sim death-stats
    read). Behaviour-preserving otherwise: with no chained burn the extra
    drain finds an empty queue and no lethal creatures. Test:
    `combat_death_whose_on_die_burns_a_bystander_settles_it_in_combat`.
  - **12.4 White Elephant — DONE.** `cards/white-elephant.lua` — white 4/4
    elephant, `2 hand + 2 attach`, the first consumer of 12.3. `on_would_die`
    sheds its sleeve and prevents the first lethal death (survives,
    sleeveless), then redirects to EXILE once sleeveless.
  - **Watch-out — re-sleeve re-arms the ward.** A sleeveless card's shed
    sleeve is attached *to* it, not *around* it, so it stays sleeveless (no
    loop). But the deferred worn/fillable-sleeves branch (putting cards into
    sleeves) would let something re-sleeve it and re-arm any "shed to
    survive" ward. Cross-branch edge to keep on the list.

## Watch-outs

- **`card_mut()` invariant — AUDITED CLEAN.** Panics on a cardless sleeve by
  design (no card to mutate). Audit of every non-test `card_mut()` call site:
  the only production path is `replay.rs::rebind_handlers`, which `continue`s
  past cardless sleeves (guarded, line 30). Runtime mutations (tapped, stats,
  modifiers) never touch card content — they go through Sleeve-level journaled
  setters and the effective-stats system, so a cardless sleeve is structurally
  never handed to `card_mut()`. The weekly stress soak (thousands of
  cardless-deck games) is the standing regression guard: any reachable
  unguarded path would panic there.
- **Visibility opacity.** `effective_top_of_deck_symbols` treats every
  cardless sleeve as transparent — correct while all are clear; gate on
  sleeve opacity once opaque colored sleeves are modeled (an opaque one
  blocks, Z.8e).

## Open questions (user input)

- Is "clear" = transparent-frame (C.13/C.14)? (Very likely yes.)
- **Deckbuilding format — RESOLVED.** A cardless sleeve is the
  `__cardless__` sentinel in a decklist / EA genome; `to_units` turns it
  into a real empty sleeve at build time.
- **C.14 for cardless sleeves — RESOLVED: any host.** Frameless → a
  non-transparent attachee, so C.14 never restricts it (already the code
  behavior; is_transparent(cardless) = false). This fires on every
  hand-cost cast paid with a cardless body (P.6 attaches it to the cast).
  Stated in Z.8d; locked by the z8c wildcard-hand-cost test.
- **EA valuation — RESOLVED: yes.** The EA drafts cardless sleeves (the
  sentinel is a first-class capped gene in random_genome / mutate /
  repair; fitness builds via to_units).

## Deferred

- **Elm UI** — out of scope this branch (rendering, free-draw animation,
  attach visuals). TODO in ELM_PLAN.md.
- **Worn + fillable sleeves** — own branch (putting cards in/out of sleeves).
- **Opaque / colored sleeves** — the sleeve-color/opacity model (colored
  transparent + opaque colored-back sleeves that carry color and can satisfy
  color costs). Beyond cardless; touches Z.8e visibility.
- **Enforcing S.4 legality** — only if/when a deck-legality check exists.
- **Deferred-event queue — DONE (slice 11).** `GameState::pending_events`
  (a transient `VecDeque`, not journaled/serialized) plus `fire_one`
  (fires one handler) + `drain_deferred_events` (fires the queue to
  empty, budget-capped). `fire_self_only` / `fire_activated` /
  `fire_with_partner` now drain after their handler unwinds. First
  consumer: `game.tap` (external taps) enqueues `OnTapped` — it fires
  once the triggering handler releases its Lua borrow, instead of not at
  all. **Second consumer — DONE:** the delayed-trigger registry
  (`GameState::delayed_triggers` + `EventName::OnDelayedTrigger` +
  `game.schedule_next_turn(iid)`); the turn loop flushes due triggers
  through `pending_events` at the scheduling player's Untap entry.
  (Shatter's counter-may turned out synchronous via `confirm_for` — it
  did not need the queue.)

## Queued tasks

- **Empty sleeve in every starter deck** (user request). **DONE for the red
  starter.** Added `genome::to_units` (`__cardless__` sentinel → `Cardless`,
  real ids → `Card`) + `shuffle_units`; both wasm start-game paths now build
  via `to_units → shuffle_units → GameState::from_units`. `RED_STARTER_DECK_IDS`
  carries 3 empty sleeves + 2 Angry Glassblowers (swapped for 5 clears).
  The blue starter still ships without empties — add the sentinel to
  `STARTER_DECK_IDS` if wanted.
- **EA drafts cardless sleeves — DONE.** The cardless sentinel is now a
  first-class draftable gene: `random_genome` can draft it, `mutate` can
  introduce/remove it, and `repair` treats it as a valid capped id (no
  longer strips it). Fitness builds each genome via `to_units` →
  `from_units`, so a drafted sentinel materializes a real empty sleeve.
  A sentinel-free genome evaluates byte-identically to the old
  `to_deck`/`new` path (same shuffle rng), and run-to-run determinism
  holds throughout — but the draft/mutate ops now include the sentinel,
  so evolved genomes for a given seed differ from before.

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

### Angry Glassblower (red creature) — DONE
- `cards/angry-glassblower.lua`. Red human, 3/4, cost 2 HAND + 1 GY.
- On attack: *may* attach an empty sleeve **from hand** to it and draw
  (resolved open question: the sleeve comes out of hand). Uses the new
  `attach_cardless_from_hand` primitive (state.rs + `game.` Lua binding).
- On dealing damage to a player: *may* exile an attached card; if it was an
  empty sleeve, draw then discard.
- Uses OnAttack + OnDealtDamageToPlayer — no OnTapped, no deferred queue.
- Shipped in the red starter (2 copies) alongside 3 loose empty sleeves.
- Tests in `game/angry_glassblower_tests.rs` (attach-from-hand, no-sleeve
  no-op, exile-empty cantrip, exile-real no-cantrip).

### Shatter Expectations (instant, colourless) — DONE (slice 10)
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
