# The Symbols of Teranos — Rules

> Working draft. Each rule has a stable identifier for review and reference.
> The document states only what has been confirmed. Inferences and CCG conventions are not assumed unless explicitly ratified.
> Rule IDs are **never renumbered**. When a rule is removed, its ID becomes a permanent gap.
>
> Canonical companion docs:
> - **`SLOTS.md`** — symbol and hole geometry (15-slot grid). When it ships in the engine, C.13 / C.14 / V.8 / V.9 are superseded by the per-slot model declared there.

## Format (F)

- **F.1** The game is played in a 1 versus 1 format.
- **F.2** There are exactly two players, each the opponent of the other.

## Setup (S)

- **S.0** Before drawing the opening HAND, each player shuffles their DECK uniformly at random. The shuffle is not observable to either player.
- **S.1** Each player starts the game with 5 cards in their HAND.
- **S.2** Each player may send up to 2 cards from their HAND to the bottom of their DECK.
- **S.3** A player who sends cards back draws an equal number of replacement cards.
- **S.4** A standard deck contains 50 sleeve-units. Most units are a sleeve holding one card; a unit may instead be a cardless sleeve (Z.8), which occupies a deck slot but is not a card.
- **S.5** A card with the subtype `test` is not legal in standard tournament play and is excluded from standard decks.
- **S.6** A standard deck contains at most 4 copies of any card (identified by `id`). Smaller pools may produce fewer copies; this is the upper bound, not a target.

## Turns (U)

- **U.1** Players alternate turns.
- **U.2** At the beginning of a player's turn, that player's tapped cards untap.
- **U.3** After untapping, that player draws cards.
- **U.4** The default number of cards drawn at the beginning of a turn is 1.
- **U.5** The active player is the player whose turn is in progress.
- **U.6** A turn consists of these phases in order: Untap, Draw, Main Phase 1, Combat, Main Phase 2, End.
- **U.7** During a Main Phase, the active player may play cards (subject to each card's timing rules) and activate abilities. The active player chooses when to leave the phase.
- **U.8** During Combat, the active player may declare attackers per B.1, B.3, B.13. The defending player may declare blockers per B.5, B.11, B.12. Damage resolves per B.7–B.9.
- **U.9** During the End Phase, end-of-turn triggers fire and accumulated damage on creatures clears (B.10).
- **U.10** During the End Phase, the active player discards down to a maximum HAND size of 6 cards. Discarded cards go to GRAVEYARD. The active player chooses which cards to discard.

## Loss (L)

- **L.1** A player loses the game when their DECK has no cards left.
- **L.2** When a player loses, their opponent wins the game.

## Zones (Z)

Zone names are written in ALL CAPS throughout the rules and the game.

The following zones are part of the game.

- **Z.1** BOARD
- **Z.2** DECK
- **Z.3** HAND
- **Z.4** EXILE
- **Z.5** GRAVEYARD
- **Z.6** ATTACHED — a card placed under another card.
- **Z.7** SAME-SLEEVE — a card fused inside the host's sleeve.
- **Z.8** CARDLESS SLEEVE — a sleeve with no card in it (an "empty sleeve"). It has no color, no symbol, and cannot be cast.
  - **Z.8a** It cannot be targeted.
  - **Z.8b** It is not a card. On top of the DECK it is taken into HAND for free and the draw continues to the next unit.
  - **Z.8c** It pays as a plain body for HAND, GRAVEYARD, and ATTACHED costs — never MILL, and never the color or symbol part of a cost (P.7a / P.12a).
  - **Z.8d** An effect can attach it to a card; while attached it can pay an ATTACHED cost.
  - **Z.8e** Nothing can be put into it.
  - **Z.8f** On top of the DECK it counts as fully transparent (V.8).

The STACK (R.8) is shared and global, not per-player, and therefore is not a Z-zone — it lives under R (Responses).

## Cards (C)

- **C.1** A card's symbols are displayed on the back of the card. A card may have zero, one, or more symbols.
- **C.2** A card is either single-sided or double-sided.
- **C.3** A card has two display states: face-up and face-down.
- **C.4** Card sleeves, when used, are always completely transparent — they exist only as card protection and never alter what's visible on either face. Coloured, patterned, or opaque sleeves are banned: the back of a card carries real game information (symbols per C.1, color per C.5, see-through windows for transparent-frame cards per C.13 + V.8) and an opaque sleeve would hide it. Most attached cards (mutations, hand-pitched payments, etc.) occupy their own sleeve and sit alongside the host as separate physical objects, which is what makes them individually movable, strippable, and targetable. A card whose printed rules specify it is fused to its host — the host and the attachment share the host's sleeve and are handled as one physical object — cannot be peeled off, targeted, or moved independently of the host; it leaves play only when the host does.
- **C.5** A card is either colorless or of one or more colors.
- **C.6** An instant is a spell with **instant timing**: it can be played at any time, including inside a response window opened by another player's action.
- **C.7** A sorcery is a spell with **sorcery timing**: it can only be played during its controller's turn, and not inside any response window. "Plain spell" (the legacy `type = "spell"` declaration with no further specialization) is treated as sorcery timing.
- **C.9** A card whose specific type is `SPELL` is non-permanent: when played, it resolves to GRAVEYARD per P.1. Instants and sorceries are spells distinguished by timing (C.6, C.7). Other card types (creature, artifact, environment) are permanents and follow their own play rules. A mutation card is permanent-like in that it remains in the game state after casting, but does not occupy a BOARD slot — it lives in its host's same-sleeve (Z.7, P.26).
- **C.10** A spell that is played resolves to GRAVEYARD. Its `on_play` handler fires after the card has left HAND and arrived in GRAVEYARD.
- **C.11** A card's symbols are structured properties that may be referenced by game effects. Each symbol on a card is independently checkable; e.g., "count cards with symbol ⨳ in your GRAVEYARD" counts every card whose symbol set contains ⨳.
- **C.12** A card's effective stats are recomputed continuously from the card's printed X/Y plus all active modifiers. Whenever game state changes, effective stats are re-evaluated.
- **C.8** A card's X/Y stats may be modified by abilities while the card is on the BOARD.
- **C.13** A card with the `transparent` **frame** cannot have any symbols — you can see through it, so there is no opaque surface on which to print them. C.1 does not apply to transparent-frame cards. Note: `transparent` is a **frame attribute**, not a color — a transparent-frame card may still carry any color identity (or none), which participates in color-matching rules as normal.
- **C.14** A `transparent`-frame card can be attached or same-sleeved only to another `transparent`-frame card. The rule applies to every attachment mechanism in the game: HAND-source payments (P.6), ATTACHED-source payments (P.31), Mutation casts (P.26, which produce same-sleeve per Z.7), and any redirected attachment (e.g., destroy-and-attach effects). Concretely: when the host (the card receiving an attachment or fusion) is non-transparent-frame, a transparent-frame attachee/mutation is refused. Transparent ↔ transparent pairings are legal. Non-transparent attachees can attach to any host. Transparent-frame cards remain valid HAND-source payments for non-BOARD casts (spells, mutations), subject to P.7a identity matching, since those payments resolve to GRAVEYARD rather than attaching.
- **C.15** A creature on the BOARD with effective Y ≤ 0 dies (placed in GRAVEYARD per P.4). This check is continuous: any change to game state that drops a creature's effective Y to 0 or below causes its death, independent of B.8's combat-damage check. P.8 cascades attached cards to EXILE as for any other BOARD → GRAVEYARD move.
- **C.16** Counting effects that reference cards "in play" or "permanents" count each BOARD card as 1. Cards attached (hand-pitched payments) or same-sleeve (mutations) to it are part of that same unit and do not contribute additional counts. The host is the unit; attached and same-sleeve cards are part of it.
- **C.17** A card may have type `symbol`. A Symbol card is a permanent — when played, it is placed on the BOARD (per P.2's analogue for symbols, see P.37). Every Symbol card is keyed by exactly one printed color **and** exactly one printed symbol; that (color, symbol) pair forms its identity. The card set commits to the full grid: for every (color, symbol) pair, exactly one Symbol card exists in the canonical pool.
- **C.17a** A Symbol card has no summoning sickness (it is a permanent of a non-creature kind, parallel to P.25 for artifacts).
- **C.17b** The backside of a Symbol card displays its printed symbol glyph occupying slot C and all eight immediately surrounding slots (UL, U, UR, L, R, DL, D, DR) — the central 3×3 region of the SLOTS.md grid. The card's color is always visible on the backside. This presentation is distinct from non-Symbol cards (C.1) and overrides their default per-slot symbol layout for Symbol cards specifically.

## Exclusions (X)

The following are not part of this game.

- **X.1** There is no mana.
- **X.2** There are no lands.

## Play (P)

- **P.1** When a card is played, it is placed in the GRAVEYARD unless a more specific rule below specifies otherwise.
- **P.2** When a creature card is played, it is placed on the BOARD.
- **P.3** A creature can only be played during your turn.
- **P.4** When a creature dies, it is placed in the GRAVEYARD.
- **P.5** If a card's cost is to exile itself, the card is placed in EXILE on play instead of its default destination from P.1 or P.2.
- **P.6** When cards from the HAND are used to pay the cost of a BOARD-placed card, those cards are attached to the played card.
- **P.7** A cost component written as `N hand` means: choose N cards from your HAND. By P.6, those cards become attached to the played card.
- **P.7a** Each HAND-source payment must *match the identity* of the card being cast. A card's identity is its set of printed colors together with its set of printed symbols. A payment matches if the two identity sets share at least one element (color overlap, or symbol overlap). A card with no colors and no symbols has empty identity. *Casting* a card with empty identity is a wildcard — it accepts any HAND payment. *Paying* with a card with empty identity is **not** a wildcard — empty cannot intersect with anything, so a no-color-no-symbol card can only pay for another no-color-no-symbol card. The identity check is independent of jewel/crystal tap substitution (P.24a/b), which has its own color-share rule. See P.12b for the case where a color-matching GRAVEYARD pitch suspends this rule for the cast.
- **P.11** A cost component written as `N mill` means: place the top N cards of your DECK into your GRAVEYARD.
- **P.12** A cost component written as `N graveyard` means: exile N cards from your GRAVEYARD. See P.12a for the identity rule on these payments.
- **P.12a** When casting a card with one or more GRAVEYARD-source cost components, at least one of the cards exiled to pay those components must share at least one *printed color* with the card being cast. The check is made once across the entire GY-payment bundle for the cast — a single color-matching pitch suffices, regardless of how many cards are exiled in total. Unlike P.7a (which checks colors ∪ symbols per HAND-source card), P.12a checks colors only — symbol overlap does not satisfy it. A card with no printed colors cannot anchor this check (empty color set cannot intersect). The `transparent` frame is not a color and does not participate in this match — anchoring is on real color identity only. Activations (A.5) do not follow P.12a; activation-time GRAVEYARD payments have no identity requirement (mirroring the HAND carve-out in A.8).
- **P.12b** When a cast's cost includes any GRAVEYARD-source component paid in compliance with P.12a, the HAND-source payments for that cast are exempt from P.7a. The color-matching GRAVEYARD pitch supplies the thematic alignment for the entire payment bundle, so any cards in the controller's HAND may pay the HAND components (subject to count, timing, and C.14's BOARD-attachment restriction on transparent cards). P.7a still governs casts whose cost has no GRAVEYARD-source component.
- **P.13** A cost component of `N hand` is valid only on cards that are placed on the BOARD when played.
- **P.16** A cost component of `sacrifice <criterion>` means: choose a card you control on the BOARD matching the criterion and move it from BOARD to GRAVEYARD as part of paying the cost.
- **P.17** A card placed in the ATTACHED zone is placed face-down.
- **P.18** The controller of an attached card may look at it at any time.
- **P.19** When an artifact is played, it is placed on the BOARD.
- **P.20** A cost component cannot be reduced below 0 by any modifier; the effective minimum is 0.
- **P.21** When an environment is played, it is placed on the BOARD.
- **P.22** At most one environment may be on the BOARD at any time, across both players.
- **P.23** A new environment cannot be played while another environment is on the BOARD.
- **P.24a** When casting a card, the controller may use one untapped card with subtype `jewel` they control on the BOARD, whose printed colors share at least one with the card being cast, to substitute for **up to two** cost components of that card. Each substituted component must be HAND-source or GRAVEYARD-source (in any combination — two HAND, two GRAVEYARD, or one of each). The jewel is tapped **and** sacrificed (BOARD → GRAVEYARD) as part of paying the cost. The jewel is consumed; it is not returned to the BOARD by any means tied to this substitution.
- **P.24b** When casting a card, the controller may tap one untapped card with subtype `crystal` they control on the BOARD if at least one card attached to that crystal shares a color with the card being cast, to substitute for exactly one HAND-source component of that card's cost.
- **P.24c** At most one P.24a, P.24b, or P.24e substitution may be made per cast. Tapping (and, for P.24a, sacrificing) is part of paying the cost.
- **P.24e** When casting a card, the controller may tap one untapped Symbol card (C.17) they control on the BOARD to substitute for exactly one HAND-source **or** one GRAVEYARD-source component of that card's cost. The Symbol card is not sacrificed; it remains on the BOARD, tapped, until normal untap (U.2).
- **P.25** A non-creature card on the BOARD has no summoning sickness restriction: it may be tapped on the turn it is played. (B.3 governs only creatures; tap-activated abilities and P.24 tap-substitutions on freshly-played artifacts are legal.)
- **P.26** A mutation card is played by targeting a creature on the BOARD. The mutation does not enter the BOARD itself: it fuses with the targeted creature in the same sleeve (Z.7). HAND-source payments for the mutation do not attach to the host — they resolve to GRAVEYARD per the spell-payment convention (C.10).
- **P.27** Any creature on either player's BOARD is a legal target for a mutation cast — friendly or opposing. The controller of the mutation cast chooses the target.
- **P.28** A mutation's effects (statics, granted keywords, granted activated abilities) apply to its host creature for as long as the host is on the BOARD. The engine resolves the mutation's static block while the host is in play, regardless of whether the host is face-up or face-down; the same-sleeve fusion does not suppress those effects.
- **P.29** When the host creature moves to any zone (GY, EXILE, HAND, DECK, or back to BOARD), its same-sleeve mutations move with it to the same zone, for the remainder of the game (Z.7). Mutations are not subject to P.8's attached-cascade to EXILE — they are fused, not attached.
- **P.8** When a card is placed in the GRAVEYARD or EXILE and it has attached cards, those attached cards are placed in EXILE.
- **P.9** When a card moves from the BOARD to a different position on the BOARD, its attached cards remain attached.
- **P.10** When a card moves from the BOARD to the HAND or to the DECK, its attached cards are placed in EXILE.
- **P.30** A variable-X cost component (a cost where `is_x = true`) has a minimum X value of 1 — X must be chosen to be at least 1. A card may explicitly opt into allowing X = 0 by declaring `allow_x_zero = true`. The default exists because X = 0 nearly always pays only the non-X cost components for a no-op effect (a strict loss); when X = 0 has a real strategic use (e.g., a stat-scaling creature played for body-less mode), the card must opt in explicitly.
- **P.31** A cost component written as `N attached` means: choose N cards currently attached to cards you control on the BOARD. If the played card is BOARD-placed, those cards become attached to the played card (face-down per P.17). If the played card is non-BOARD, those cards are placed in EXILE. P.7a does not apply to ATTACHED-source payments.
- **P.32** A card with a declared target category cannot be cast when no legal target for that category exists. The engine refuses the cast before any cost is paid. Target categories are declarative on the card (e.g., `target = "chain"`), and the engine's legality check is built-in per category — parallel to A.9 for activations.
- **P.33** When a card is cast, the card itself leaves HAND immediately at cast announcement and enters the STACK (R.8). The STACK is not a zone; see R.8. It does not stay in HAND while the STACK resolves. Consequence: the same card cannot be cast twice while it sits on the STACK — once cast, it is no longer in HAND to be selected again. On resolution the STACK item moves to its destination zone per the card type (creature/artifact → BOARD per P.2/P.19; spell → GRAVEYARD per C.10; mutation → ATTACHED per P.26). If countered, the STACK item moves to GRAVEYARD (its cast attempt did not resolve, but the card is no longer in HAND).
- **P.34** There is no limit on the number of cards a player may cast per turn, per main phase, or per card type. A player may cast as many creatures, artifacts, environments (subject to P.22's single-environment-on-board limit), spells, and mutations as their resources and timing rules (C.6, C.7, P.3) permit. Only payment availability, target availability (P.32), and zone constraints limit how many cards leave HAND each turn. Symbol cards (C.17) are the sole exception — see P.35.
- **P.35** A player may cast at most one Symbol card per turn. This cap is per-player, per-turn, and counts across both main phases.
- **P.36** A Symbol card is unique in play: if a Symbol card with a given `id` is on either player's BOARD, no player may cast another Symbol card with that same `id`. This is a casting restriction, not a state replacement — the second cast is refused before any cost is paid. When the first leaves the BOARD by any means, the same `id` becomes castable again. The uniqueness is by card `id`, so e.g. "Red IX" and "Blue IX" are distinct ids and can coexist; two "Red IX" cards cannot.
- **P.37** When a Symbol card is played, it is placed on the BOARD untapped. This overrides any default tap-state that would otherwise apply.
- **P.38** When a Symbol card is on top of a player's DECK, the controller of that DECK may cast it from the top of the DECK as if it were in their HAND, subject to all normal cost, timing, and uniqueness rules (P.32, P.35, P.36, C.6/C.7 timing). V.1 already makes the top card's symbols visible, but P.38 grants the additional cast-zone affordance specifically for Symbol cards. The cast pays its cost normally; on resolution the card moves from DECK to BOARD per P.37 (skipping HAND).
- **P.39** A Clear card (the canonical `clear-*` cycle) has a search effect that lets its controller search their DECK for either (a) a card with subtype `jewel`, or (b) a Symbol card sharing at least one printed color with the Clear, and place the chosen card into their HAND. The DECK is then shuffled. If no eligible card exists in the DECK, the effect produces no card but the cast still resolves.

## Abilities (A)

- **A.1** A card may have triggered abilities. A triggered ability fires when a specified event occurs.
- **A.2** A card may have static abilities. A static ability is a continuous effect that applies while the source card is on the BOARD.
- **A.3** When an effect specifies a target, the player playing the effect chooses the specific card or player to which the effect applies.
- **A.4** An effect of `draw N` means: move the top N cards of the controller's DECK to their HAND.
- **A.5** A card may have **activated abilities**. The controller may fire one in their main phase by paying the listed cost. The effect resolves immediately — activations do not go on the stack and cannot be responded to. This is a deliberate departure from MTG; the trade is simplicity (no nested priority windows around activations) at the cost of "kill the source before the ability fires" plays.
- **A.6** The notation `T:` ("tap") is an activation cost. The source card must be on its controller's BOARD and untapped; after the cost is paid the source becomes tapped (B.4). For creature sources, B.3 summoning sickness applies — the source must have been on the BOARD since at least the start of its controller's previous turn, unless it has `haste`. Vigilance does not exempt a creature from being tapped by `T:` activation; it only exempts the creature from tapping when attacking (B.4).
- **A.7** A creature attacking (B.1) sets a per-instance `attacked_this_turn` flag that activations may read. The flag is cleared at the start of each turn. Used by abilities like `T: if this creature attacked this turn, …` to distinguish "attacked + activated" from "just activated."
- **A.8** Activation costs are a combination of: an optional tap component (`T:` per A.6) and zero or more play-card cost components in the existing vocabulary — HAND (discarded cards), MILL (top of own deck → graveyard), or GRAVEYARD (moved to exile per the play-card convention). SACRIFICE and SELF components are not yet supported by activations and are reserved. HAND payments for activations do **not** follow P.7a (the HAND-identity rule). That restriction governs casting only; activation hand-pay can use any cards from hand. Components are paid before the activation's effect resolves; pre-validation guarantees the payment loop cannot fail mid-way.
- **A.9** An activated ability may only be initiated when its target requirements are satisfiable — if the effect requires a target and no legal one exists, the controller cannot activate (and pays no cost). Card authors declare this gate via an optional `validate` Lua hook on the ability. Absence of `validate` means the ability has no target requirement (e.g., "draw a card" / "creatures you control get +2/+2"). The sim AI uses this gate to avoid burning resources on no-op activations.
- **A.10** An activated ability may be granted to a card by a static effect on another card (e.g., a jewel pitched as cost grants `T: draw, discard` to the creature it attached to). The granted ability appears alongside any printed activations on the recipient and follows the same A.5–A.9 semantics. The cost is paid by the recipient (the host taps for a granted `T:`, not the source), and the effect resolves with the recipient as the activation source.
- **A.11** **P/T modifications resolve before other effects.** Whenever a triggered, activated, or static effect reads a creature's `X` or `Y` stat, it observes the effective post-modifier value — i.e., all stat-modifying statics, attached buffs, debuffs, and modifier-stacking adjustments have already applied at the moment of the read. Card authors read stats via the engine's effective-stat path (`game.card(iid).x` / `.y`); never recompute from printed values. This ordering means a same-turn buff is visible to a same-turn trigger, and a creature whose toughness was just modified mid-turn reflects the new value to any subsequent on_attack / on_block / on_turn_begin / activated reads.
- **A.12** **Cost reductions resolve before other effects.** Whenever a triggered, activated, or static effect reads a card's `combined_cost` (or any other cost field), it observes the effective post-reduction value — i.e., every cost-reducing static and modifier has already applied at the moment of the read. Same shape as A.11: the only Lua-exposed accessor goes through the engine's effective-cost path; printed cost is not reachable from a Lua handler. So a handler-gate like "destroy creature with combined cost ≥ N" reads the *reduced* cost when the target sits under a cost-reduction static, matching the symmetric expectation set by A.11 for toughness buffs.

## Control (T)

- **T.1** BOARD is a per-player zone: each player has their own BOARD. A card on a player's BOARD is controlled by that player. Cards are added to and removed from a specific player's BOARD; "the BOARD" as a phrase refers to either player's BOARD individually, never a shared container.
- **T.2** Every card has an owner: the player to whose initial DECK it belonged. Ownership does not change during the game.

## Responses (R)

- **R.1** A response window — a period during which both players may play a card (subject to its timing) or pass, before the triggering event resolves — opens when (a) a card is played, or (b) an attack is declared. Outside these moments, actions and events resolve atomically.
- **R.2** Responses resolve in reverse order: the most recently added response resolves first.
- **R.3** A player may play a card as a response only if its normal timing permits it at that moment.
- **R.4** A response is itself an action and may also be responded to.
- **R.5** When both players consecutively pass, the most recently added unresolved item in the response chain resolves.
- **R.6** When both players consecutively pass and the response chain is empty, the response window closes.
- **R.7** When a response window opens, the active player (the one whose turn it is) acts first. They may respond by playing a card, or pass. Their opponent only gets a chance to act after the active player passes or responds.
- **R.8** The STACK is the shared, global ordered list of cast cards waiting to resolve. It is NOT a zone (Z.1–Z.6 are per-player; the STACK is global and singular). A cast card enters the STACK at announcement per P.33 and leaves it on resolution or counter. Each STACK item carries the controller (who cast it). Resolution is LIFO per R.2.

## Visibility (V)

- **V.1** In a DECK, the top card's symbols are visible to both players.
- **V.2** In a DECK, all cards except the top are concealed (including the bottom and any cards between).
- **V.3** Cards in a player's HAND are fully visible to that player and concealed from their opponent.
- **V.4** Cards in the GRAVEYARD are fully visible to both players.
- **V.5** Cards in EXILE are fully visible to both players.
- **V.6** Cards on the BOARD are fully visible to both players.
- **V.7** Visibility of cards in ATTACHED is defined by P.17 (face-down, symbols visible to both players) and P.18 (controller may look at the face at any time).
- **V.7b** Same-sleeve cards (Z.7) share the host's visibility state — they are not individual game objects, so they have no independent visibility. Whatever zone the host is in, the same-sleeve cards are shown per that zone's rules; their controller may look at the face at any time per P.18's principle.
- **V.8** A `transparent`-frame card on top of a DECK reveals the symbols of the card immediately below it. The card below is seen through the transparent-frame card, which means players see its back; per C.1 the back is where the symbols are. If the revealed card is itself transparent-frame, V.8 applies recursively to the card below it, continuing until an opaque card is reached.
- **V.9** A card with the `glow` face attribute has visibility determined by its **effective slot** in the DECK, computed by counting only non-transparent-frame cards above it. Transparent-frame cards in slots above are ignored for this computation. Glow is a cosmetic surface treatment (see `Card.face`), not a color — granted via `static.modifier.face = {"glow"}` and read via `effective_face(iid)`.
- **V.10** A `glow` card at effective slot 0 is fully visible to both players (all properties).
- **V.11** A `glow` card at effective slot 1 has its color and type visible to both players; other properties remain concealed. Cards at effective slot 2 or deeper are concealed normally.

## Combat (B)

- **B.1** A creature can attack a player.
- **B.2** When a creature attacks a player successfully, that player exiles X cards from their DECK, where X is the first value in the creature's X/Y stats.
- **B.2b** When X is fractional (allowed since the fractional-stats refactor), B.2 reads as ⌊ΣX⌋ per combat: the mill is the floor of the sum of X across every successful attacker (B.6) in the same combat — not per-attacker. So one 0.5/1 unblocked mills 0, two 0.5/1 unblocked sum to 1.0 and mill 1, a 0.5/1 + a 1/1 unblocked sum to 1.5 and mill 1.
- **B.3** A creature cannot attack during the turn it enters the BOARD, regardless of how it entered.
- **B.4** When a creature attacks, its card is tapped (turned sideways).
- **B.5** During combat, the defending player may declare one or more of their creatures as blockers, each assigned to a specific attacking creature.
- **B.6** An attack on a player is "successful" (per B.2) if and only if it is not blocked.
- **B.7** When an attacker is blocked, the attacker deals damage equal to its X to each of its blockers, and each blocker deals damage equal to its X to the attacker. Damage accumulates fractionally — two 0/0.5 blockers (each dealing 0.5) on a 1/1 attacker accumulate 1.0 damage, which equals the attacker's Y and kills it per B.8.
- **B.8** A creature with accumulated damage equal to or greater than its Y dies (placed in GRAVEYARD per P.4). Both damage and Y are real-valued; the comparison is exact (no rounding).
- **B.10** At the end of the turn, all accumulated damage on creatures is cleared.
- **B.11** **Flying** is a keyword ability: a creature with flying can only be blocked by a card with flying, or by a card whose text explicitly grants the ability to block flying.
- **B.12** A tapped creature cannot block.
- **B.13** A tapped creature cannot attack.
- **B.14** **Unblockable** is a keyword ability: a creature with this keyword cannot be blocked.
- **B.15** **Haste** is a keyword ability: a creature with this keyword may attack the turn it enters the BOARD, overriding B.3.
- **B.16** **Vigilance** is a keyword ability: a creature with this keyword does not tap when it attacks, overriding B.4.
- **B.17** **Defender** is a keyword ability: a creature with this keyword cannot attack.
- **B.18** **cannot-block** is a keyword ability: a creature with this keyword cannot be declared as a blocker. The mirror of defender — defender prevents attacking, cannot-block prevents blocking.
