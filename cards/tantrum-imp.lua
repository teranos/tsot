return {
  id = "tantrum-imp",
  name = "Tantrum Imp",
  type = "creature",
  colors = {"red"},
  subtypes = {"imp"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "Whenever Tantrum Imp becomes blocked, the blocker takes 1 damage and the defending player mills 1 card to EXILE.",
  },
  stats = {x = 1, y = 2},
  on_blocked_by = function(game, self, blocker)
    game.damage(blocker.instance_id, 1)
    game.mill(game.opponent(self.owner), 1, "exile")
  end,
}
