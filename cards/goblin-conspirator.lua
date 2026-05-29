-- Black goblin of the cycle: 1/1, 1 hand + 2 mill, on-play reveal + draw.
-- Handler deferred: the "may reveal another goblin card from your hand"
-- needs both the choice API (Phase 2) and `game.zones(self.owner).hand`
-- with type-filtering. Both Phase 2/3 work.
return {
  id = "goblin-conspirator",
  name = "Goblin Conspirator",
  colors = {"black"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "when you play this card you may reveal another goblin card from your hand. when you do, draw a card.",
  },
  stats = {x = 1, y = 1},
}
