return {
  id = "avatar-of-greed",
  name = "Avatar of Greed",
  colors = {"blue", "black", "orange"},
  type = "creature",
  subtypes = {"avatar"},
  symbols = {"⨳", "⋈", "≡"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 4, source = "attached"},
    {amount = 8, source = "graveyard"},
  },
  abilities = {
    "whenever a creature dies, you may draw a card. if you do, every player mills 2.",
  },
  stats = {x = 8, y = 8},
  on_zone_change = function(game, self, moving, from, to)
    if from ~= "board" or to ~= "graveyard" then return end
    if moving.instance_id == self.instance_id then return end
    local c = game.card(moving.instance_id)
    if not c or c.type ~= "creature" then return end
    if not game.confirm("draw a card? (every player mills 2)") then return end
    game.draw(self.owner, 1)
    game.mill(self.owner, 2, "graveyard")
    game.mill(game.opponent(self.owner), 2, "graveyard")
  end,
}
