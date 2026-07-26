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
  on_enter_board = function(game, self)
    game.draw(self.owner, 1)
  end,
}
