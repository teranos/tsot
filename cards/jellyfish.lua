-- Symbol not yet specified.
return {
  id = "jellyfish",
  name = "Jellyfish",
  colors = {"blue"},
  type = "creature",
  subtypes = {"fish"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
    {amount = 3, source = "graveyard"},
  },
  abilities = {
    "When this creature enters the board, return target creature to its owners hand.",
  },
  stats = {x = 0, y = 1},
}
