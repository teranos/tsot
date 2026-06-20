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
    {amount = 4, source = "graveyard"},
  },
  abilities = {
    "this creature attacks every turn if able.",
    "this creature gets +X/+Y where X is the number of distinct card types in play (subtypes excluded) and Y is the number of cards in players' hands.",
  },
  stats = {x = 0, y = 0},
  static = {
    affects = {scope = "source_only"},
    modifier = {x = "board_types", y = "hands"},
  },
}

