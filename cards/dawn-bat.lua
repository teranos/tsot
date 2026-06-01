-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "dawn-bat",
  name = "Dawn Bat",
  type = "creature",
  colors = {"white"},
  subtypes = {"bat"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 3, y = 1},
  abilities = {
    "flying.",
    "whenever this creature attacks, mill 1.",
  },
  on_attack = function(game, self)
    game.mill(self.owner, 1, "graveyard")
  end,
}
