-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "witch-bat",
  name = "Witch Bat",
  type = "creature",
  colors = {"purple"},
  subtypes = {"bat"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  stats = {x = 3, y = 1},
  abilities = {
    "flying.",
    "whenever this creature attacks, mill 1.",
  },
  on_attack = function(game, self)
    game.mill(self.owner, 1, "graveyard")
  end,
}
