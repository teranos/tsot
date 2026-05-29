return {
  id = "primal-toad",
  name = "Primal Toad",
  symbol = "⊨",
  colors = {"green"},
  type = "creature",
  subtypes = {"toad"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
    {amount = 3, source = "graveyard"},
  },
  abilities = {
    "this creature attacks every turn if able.",
    "this creature gets +X/+Y where X is the number of cards in play and Y is the number of cards in players' hands.",
  },
  stats = {x = 0, y = 0},
}
