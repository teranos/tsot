-- Purple goblin — free 0/0. Designed to combo with goblin lord effects:
-- a goblin anthem turns the 0/0 into a real threat for zero cost.
--
-- Handler deferred — needs three things we don't have:
--   1. `game.discard(player_id, n)`: lands (deterministic front-of-hand) but
--      the natural reading is "which card to discard?" — full choice arrives
--      in Phase 2 when `game.choose_card` is widely adopted.
--   2. A counter/modifier-add API (e.g. `game.add_modifier(iid, "+1/+1")`):
--      `Modifier::StatBoost` exists in state.rs but isn't exposed to handlers.
--   3. The "may" pattern uses `game.confirm("Discard for +1/+1?")`.
--
-- Until then, the card lands as a free 0/0 with no on_enter_board effect;
-- still useful as a chump blocker or anthem fodder.
return {
  id = "eager-goblin",
  name = "Eager Goblin",
  colors = {"purple"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {},
  abilities = {
    "when this creature enters the board, you may discard a card. if you do, this creature enters with a +1/+1 counter.",
  },
  stats = {x = 0, y = 0},
}
