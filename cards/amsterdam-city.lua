return {
  id = "amsterdam-city",
  name = "Amsterdam City",
  symbol = "≡",
  colors = {"black"},
  type = "environment",
  subtypes = {"Urban"},
  cost = {
    {amount = 4, source = "mill"},
    {amount = 4, source = "graveyard"},
  },
  abilities = {
    "4+ cost for deck and graveyard cast cost.",
    "you can see which cards are red and black.",
    "if there are 3 or more ix cards across both players' graveyards, either player may (when they have priority) exile those ix cards and this card. each opponent then sacrifices a creature, discards 2 cards, exiles the top 3 cards of their deck and exiles 4 cards from their graveyard.",
  },
}
