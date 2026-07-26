-- Blue goblin of the cycle: 1/1, 1 hand + 2 mill, ETB draw 1.
return {
  id = "goblin-scribe",
  name = "Goblin Scribe",
  colors = {"blue"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "when this creature enters the board, draw a card.",
  },
  stats = {x = 1, y = 1},
  on_zone_change = function(game, self, moving, from, to)
    if moving.instance_id ~= self.instance_id then return end
    if to ~= "board" then return end
    if game.host_of(self.instance_id) then return end
    game.draw(self.owner, 1)
  end,
}
