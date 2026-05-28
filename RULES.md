# The Symbols of Teranos — Rules

> Working draft. Each rule has a stable identifier for review and reference.
> The document states only what has been confirmed. Inferences and CCG conventions are not assumed unless explicitly ratified.

## Format (F)

- **F.1** The game is played in a 1 versus 1 format.
- **F.2** There are exactly two players, each the opponent of the other.

## Setup (S)

- **S.1** Each player starts the game with 5 cards in their HAND.
- **S.2** Each player may send up to 2 cards from their HAND to the bottom of their DECK.
- **S.3** A player who sends cards back draws an equal number of replacement cards.

## Turns (U)

- **U.1** Players alternate turns.
- **U.2** At the beginning of a player's turn, that player's tapped cards untap.
- **U.3** After untapping, that player draws cards.
- **U.4** The default number of cards drawn at the beginning of a turn is 1.
- **U.5** The active player is the player whose turn is in progress.

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
- **C.6** An instant is a card that can be played at any time.
- **C.9** Every card is a spell. The card's specific type (e.g., creature, instant, spell, artifact) is a kind of spell.
- **C.10** A card whose specific type is `SPELL` (with no further specialization) can only be played during its controller's turn.
- **C.11** A card's symbol is a structured property that may be referenced by game effects (e.g., "count cards with symbol ⨳ in your GRAVEYARD").
- **C.7** A wall is a card type, distinct from creature and instant.
- **C.8** A card's X/Y stats may be modified by abilities while the card is on the BOARD.

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
- **P.11** A cost component written as `N mill` means: place the top N cards of your DECK into your GRAVEYARD.
- **P.12** A cost component written as `N graveyard` means: exile N cards from your GRAVEYARD.
- **P.13** A cost component of `N hand` is valid only on cards that are placed on the BOARD when played.
- **P.14** When a wall is played, it is placed on the BOARD.
- **P.15** A wall does not die. Triggers and effects that reference creature death do not apply to walls.
- **P.16** A cost component of `sacrifice <criterion>` means: choose a card you control on the BOARD matching the criterion and move it from BOARD to GRAVEYARD as part of paying the cost.
- **P.17** A card placed in the ATTACHED zone is placed face-down.
- **P.18** The controller of an attached card may look at it at any time.
- **P.19** When an artifact is played, it is placed on the BOARD.
- **P.20** A cost component cannot be reduced below 0 by any modifier; the effective minimum is 0.
- **P.21** When an environment is played, it is placed on the BOARD.
- **P.22** At most one environment may be on the BOARD at any time, across both players.
- **P.23** A new environment cannot be played while another environment is on the BOARD.
- **P.8** When a card is placed in the GRAVEYARD or EXILE and it has attached cards, those attached cards are placed in EXILE.
- **P.9** When a card moves from the BOARD to a different position on the BOARD, its attached cards remain attached.
- **P.10** When a card moves from the BOARD to the HAND or to the DECK, its attached cards are placed in EXILE.

## Abilities (A)

- **A.1** A card may have triggered abilities. A triggered ability fires when a specified event occurs.
- **A.2** A card may have static abilities. A static ability is a continuous effect that applies while the source card is on the BOARD.
- **A.3** When an effect specifies a target, the player playing the effect chooses the specific card or player to which the effect applies.
- **A.4** An effect of `draw N` means: move the top N cards of the controller's DECK to their HAND.

## Control (T)

- **T.1** Every card on the BOARD is controlled by exactly one player. That player is the card's controller.
- **T.2** Every card has an owner: the player to whose initial DECK it belonged. Ownership does not change during the game.

## Responses (R)

- **R.1** A response window opens when (a) a card is played, or (b) an attack is declared. Outside these moments, actions and events resolve atomically.
- **R.2** Responses resolve in reverse order: the most recently added response resolves first.
- **R.3** A player may play a card as a response only if its normal timing permits it at that moment.
- **R.4** A response is itself an action and may also be responded to.
- **R.5** When both players consecutively pass, the most recently added unresolved item in the response chain resolves.
- **R.6** When both players consecutively pass and the response chain is empty, the response window closes.
- **R.7** Within a response window, the active player has the first opportunity to respond or pass.

## Visibility (V)

- **V.1** In a DECK, the top card's symbol is visible to both players.
- **V.2** In a DECK, all cards except the top are concealed (including the bottom and any cards between).
- **V.3** Cards in a player's HAND are fully visible to that player and concealed from their opponent.
- **V.4** Cards in the GRAVEYARD are fully visible to both players.
- **V.5** Cards in EXILE are fully visible to both players.
- **V.6** Cards on the BOARD are fully visible to both players.
- **V.7** Visibility of cards in ATTACHED is defined by P.17 (face-down, symbol visible to both players) and P.18 (controller may look at the face at any time).

## Combat (B)

- **B.1** A creature can attack a player.
- **B.2** When a creature attacks a player successfully, that player exiles X cards from their DECK, where X is the first value in the creature's X/Y stats.
- **B.3** A creature cannot attack during the turn it enters the BOARD, regardless of how it entered.
- **B.4** When a creature attacks, its card is tapped (turned sideways).
- **B.5** During combat, the defending player may declare one or more of their creatures or walls as blockers, each assigned to a specific attacking creature.
- **B.6** An attack on a player is "successful" (per B.2) if and only if it is not blocked.
- **B.7** When an attacker is blocked, the attacker deals damage equal to its X to each of its blockers, and each blocker deals damage equal to its X to the attacker.
- **B.8** A creature with accumulated damage equal to or greater than its Y dies (placed in GRAVEYARD per P.4).
- **B.9** Walls do not die from damage (per P.15). Accumulated damage on a wall is cleared at the end of combat.
- **B.10** At the end of the turn, all accumulated damage on creatures is cleared.
- **B.11** A flying creature can only be blocked by a card with flying, or by a card whose text explicitly grants it the ability to block flying.
- **B.12** A tapped creature cannot block.
- **B.13** A tapped creature cannot attack.
