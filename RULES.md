# The Symbols of Teranos — Rules

> Working draft. Each rule has a stable identifier for review and reference.
> The document states only what has been confirmed. Inferences and CCG conventions are not assumed unless explicitly ratified.
> Rule IDs are **never renumbered**. When a rule is removed, its ID becomes a permanent gap.

## Format (F)

- **F.1** The game is played in a 1 versus 1 format.
- **F.2** There are exactly two players, each the opponent of the other.

## Setup (S)

- **S.1** Each player starts the game with 5 cards in their HAND.
- **S.2** Each player may send up to 2 cards from their HAND to the bottom of their DECK.
- **S.3** A player who sends cards back draws an equal number of replacement cards.
- **S.4** A standard deck contains 50 cards.
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

## Cards (C)

- **C.1** A card's symbol is displayed on the back of the card.
- **C.2** A card is either single-sided or double-sided.
- **C.3** A card has two display states: face-up and face-down.
- **C.4** Sleeves are not part of this game. Sleeves are permanently banned.
- **C.5** A card is either colorless or of one or more colors.
- **C.6** An instant is a spell with **instant timing**: it can be played at any time, including inside a response window opened by another player's action.
- **C.7** A sorcery is a spell with **sorcery timing**: it can only be played during its controller's turn, and not inside any response window. "Plain spell" (the legacy `type = "spell"` declaration with no further specialization) is treated as sorcery timing.
- **C.9** A card whose specific type is `SPELL` is non-permanent: when played, it resolves to GRAVEYARD per P.1. Instants and sorceries are spells distinguished by timing (C.6, C.7). Other card types (creature, artifact, environment) are permanents and follow their own play rules. A mutation card is permanent-like in that it remains in the game state after casting, but does not occupy a BOARD slot — it lives in its host's attached zone (P.26).
- **C.10** A spell that is played resolves to GRAVEYARD. Its `on_play` handler fires after the card has left HAND and arrived in GRAVEYARD.
- **C.11** A card's symbol is a structured property that may be referenced by game effects (e.g., "count cards with symbol ⨳ in your GRAVEYARD").
- **C.12** A card's effective stats are recomputed continuously from the card's printed X/Y plus all active modifiers. Whenever game state changes, effective stats are re-evaluated.
- **C.8** A card's X/Y stats may be modified by abilities while the card is on the BOARD.
- **C.13** A card with the `transparent` color cannot have a symbol — you can see through it, so there is no opaque surface on which to print one. C.1 does not apply to transparent cards.

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
- **P.7a** Each HAND-source payment must *match the identity* of the card being cast. A card's identity is its set of printed colors together with its `symbol` (if non-empty). A payment matches if the two identity sets share at least one element (color overlap, or matching symbol). A card with no colors and no symbol has empty identity. *Casting* a card with empty identity is a wildcard — it accepts any HAND payment. *Paying* with a card with empty identity is **not** a wildcard — empty cannot intersect with anything, so a no-color-no-symbol card can only pay for another no-color-no-symbol card. The identity check is independent of jewel/crystal tap substitution (P.24a/b), which has its own color-share rule.
- **P.11** A cost component written as `N mill` means: place the top N cards of your DECK into your GRAVEYARD.
- **P.12** A cost component written as `N graveyard` means: exile N cards from your GRAVEYARD.
- **P.13** A cost component of `N hand` is valid only on cards that are placed on the BOARD when played.
- **P.16** A cost component of `sacrifice <criterion>` means: choose a card you control on the BOARD matching the criterion and move it from BOARD to GRAVEYARD as part of paying the cost.
- **P.17** A card placed in the ATTACHED zone is placed face-down.
- **P.18** The controller of an attached card may look at it at any time.
- **P.19** When an artifact is played, it is placed on the BOARD.
- **P.20** A cost component cannot be reduced below 0 by any modifier; the effective minimum is 0.
- **P.21** When an environment is played, it is placed on the BOARD.
- **P.22** At most one environment may be on the BOARD at any time, across both players.
- **P.23** A new environment cannot be played while another environment is on the BOARD.
- **P.24a** When casting a card, the controller may tap one untapped card with subtype `jewel` they control on the BOARD whose printed colors share at least one with the card being cast, to substitute for exactly one HAND-source component of that card's cost.
- **P.24b** When casting a card, the controller may tap one untapped card with subtype `crystal` they control on the BOARD if at least one card attached to that crystal shares a color with the card being cast, to substitute for exactly one HAND-source component of that card's cost.
- **P.24c** At most one P.24a or P.24b tap-substitution may be made per cast. Tapping is part of paying the cost.
- **P.25** A non-creature card on the BOARD has no summoning sickness restriction: it may be tapped on the turn it is played. (B.3 governs only creatures; tap-activated abilities and P.24 tap-substitutions on freshly-played artifacts are legal.)
- **P.26** A mutation card is played by targeting a creature on the BOARD. The mutation does not enter the BOARD itself: it attaches to the targeted creature (face-down per P.17), and remains attached for as long as that creature is on the BOARD. HAND-source payments for the mutation do not attach to the host — they resolve to GRAVEYARD per the spell-payment convention (C.10).
- **P.27** Any creature on either player's BOARD is a legal target for a mutation cast — friendly or opposing. The controller of the mutation cast chooses the target.
- **P.28** A mutation's effects (statics, granted keywords, granted activated abilities) apply to its host creature for as long as the mutation is attached. P.17's face-down state does not suppress those effects: the engine still resolves the mutation's static block while it is in the host's attached zone.
- **P.29** If the host creature leaves the BOARD by any means (death, control change to a non-BOARD zone, return to hand, exile, etc.), the attached mutation is placed in EXILE per P.8.
- **P.8** When a card is placed in the GRAVEYARD or EXILE and it has attached cards, those attached cards are placed in EXILE.
- **P.9** When a card moves from the BOARD to a different position on the BOARD, its attached cards remain attached.
- **P.10** When a card moves from the BOARD to the HAND or to the DECK, its attached cards are placed in EXILE.

## Abilities (A)

- **A.1** A card may have triggered abilities. A triggered ability fires when a specified event occurs.
- **A.2** A card may have static abilities. A static ability is a continuous effect that applies while the source card is on the BOARD.
- **A.3** When an effect specifies a target, the player playing the effect chooses the specific card or player to which the effect applies.
- **A.4** An effect of `draw N` means: move the top N cards of the controller's DECK to their HAND.
- **A.5** A card may have **activated abilities**. The controller may fire one in their main phase by paying the listed cost. The effect resolves immediately — activations do not go on the stack and cannot be responded to. This is a deliberate departure from MTG; the trade is simplicity (no nested priority windows around activations) at the cost of "kill the source before the ability fires" plays.
- **A.6** The notation `T:` ("tap") is an activation cost. The source card must be on its controller's BOARD and untapped; after the cost is paid the source becomes tapped (B.4). For creature sources, B.3 summoning sickness applies — the source must have been on the BOARD since at least the start of its controller's previous turn, unless it has `haste`. Vigilance does not exempt a creature from being tapped by `T:` activation; it only exempts the creature from tapping when attacking (B.4).
- **A.7** A creature attacking (B.1) sets a per-instance `attacked_this_turn` flag that activations may read. The flag is cleared at the start of each turn. Used by abilities like `T: if this creature attacked this turn, …` to distinguish "attacked + activated" from "just activated."

## Control (T)

- **T.1** Every card on the BOARD is controlled by exactly one player. That player is the card's controller.
- **T.2** Every card has an owner: the player to whose initial DECK it belonged. Ownership does not change during the game.

## Responses (R)

- **R.1** A response window — a period during which both players may play a card (subject to its timing) or pass, before the triggering event resolves — opens when (a) a card is played, or (b) an attack is declared. Outside these moments, actions and events resolve atomically.
- **R.2** Responses resolve in reverse order: the most recently added response resolves first.
- **R.3** A player may play a card as a response only if its normal timing permits it at that moment.
- **R.4** A response is itself an action and may also be responded to.
- **R.5** When both players consecutively pass, the most recently added unresolved item in the response chain resolves.
- **R.6** When both players consecutively pass and the response chain is empty, the response window closes.
- **R.7** When a response window opens, the active player (the one whose turn it is) acts first. They may respond by playing a card, or pass. Their opponent only gets a chance to act after the active player passes or responds.

## Visibility (V)

- **V.1** In a DECK, the top card's symbol is visible to both players.
- **V.2** In a DECK, all cards except the top are concealed (including the bottom and any cards between).
- **V.3** Cards in a player's HAND are fully visible to that player and concealed from their opponent.
- **V.4** Cards in the GRAVEYARD are fully visible to both players.
- **V.5** Cards in EXILE are fully visible to both players.
- **V.6** Cards on the BOARD are fully visible to both players.
- **V.7** Visibility of cards in ATTACHED is defined by P.17 (face-down, symbol visible to both players) and P.18 (controller may look at the face at any time).
- **V.8** A `transparent` card on top of a DECK reveals the symbol of the card immediately below it. The card below is seen through the transparent card, which means players see its back; per C.1 the back is where the symbol is. If the revealed card is itself `transparent`, V.8 applies recursively to the card below it, continuing until an opaque card is reached.
- **V.9** A `glow` card's visibility is determined by its **effective slot** in the DECK, computed by counting only non-`transparent` cards above it. Transparent cards in slots above are ignored for this computation.
- **V.10** A `glow` card at effective slot 0 is fully visible to both players (all properties).
- **V.11** A `glow` card at effective slot 1 has its color and type visible to both players; other properties remain concealed. Cards at effective slot 2 or deeper are concealed normally.

## Combat (B)

- **B.1** A creature can attack a player.
- **B.2** When a creature attacks a player successfully, that player exiles X cards from their DECK, where X is the first value in the creature's X/Y stats.
- **B.3** A creature cannot attack during the turn it enters the BOARD, regardless of how it entered.
- **B.4** When a creature attacks, its card is tapped (turned sideways).
- **B.5** During combat, the defending player may declare one or more of their creatures as blockers, each assigned to a specific attacking creature.
- **B.6** An attack on a player is "successful" (per B.2) if and only if it is not blocked.
- **B.7** When an attacker is blocked, the attacker deals damage equal to its X to each of its blockers, and each blocker deals damage equal to its X to the attacker.
- **B.8** A creature with accumulated damage equal to or greater than its Y dies (placed in GRAVEYARD per P.4).
- **B.10** At the end of the turn, all accumulated damage on creatures is cleared.
- **B.11** **Flying** is a keyword ability: a creature with flying can only be blocked by a card with flying, or by a card whose text explicitly grants the ability to block flying.
- **B.12** A tapped creature cannot block.
- **B.13** A tapped creature cannot attack.
- **B.14** **Unblockable** is a keyword ability: a creature with this keyword cannot be blocked.
- **B.15** **Haste** is a keyword ability: a creature with this keyword may attack the turn it enters the BOARD, overriding B.3.
- **B.16** **Vigilance** is a keyword ability: a creature with this keyword does not tap when it attacks, overriding B.4.
- **B.17** **Defender** is a keyword ability: a creature with this keyword cannot attack.
- **B.18** **cannot-block** is a keyword ability: a creature with this keyword cannot be declared as a blocker. The mirror of defender — defender prevents attacking, cannot-block prevents blocking.
