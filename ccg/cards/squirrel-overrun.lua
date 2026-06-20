-- Symbol not yet specified.
return {
  id = "squirrel-overrun",
  name = "Squirrel Overrun",
  colors = {"green"},
  type = "creature",
  subtypes = {"Squirrel"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "this creature attacks every turn if able.",
    "whenever this creature attacks you may attach 1.",
    "this creature gets +1/+1 for each attached card.",
    "whenever another creature blocks this creature, draw a card.",
  },
  stats = {x = 0, y = 0},
  on_blocked_by = function(game, self, blocker)
    game.draw(self.owner, 1)
  end,
}
