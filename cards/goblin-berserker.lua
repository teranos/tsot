-- Red goblin of the cycle: 1/1, 1 hand + 2 mill, on attack discard 1 + draw 1.
-- Handler deferred: "discard a card" requires choice (which card from hand);
-- choice API is LUA Phase 2.
return {
  id = "goblin-berserker",
  name = "Goblin Berserker",
  colors = {"red"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "whenever this creature attacks, discard a card and draw a card.",
  },
  stats = {x = 1, y = 1},
}
