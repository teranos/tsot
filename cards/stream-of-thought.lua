-- Color and symbol not yet specified.
-- Conflict: P.13 says `N hand` is only valid on cards placed on the BOARD when played.
-- This card is an INSTANT and would not go to BOARD. Resolution pending.
return {
  id = "stream-of-thought",
  name = "Stream of Thought",
  colors = {"blue", "transparent"},
  type = "instant",
  cost = {{amount = 2, source = "hand"}},
  abilities = {
    "draw 3 cards. put one card from your hand back on top of your DECK.",
  },
}
