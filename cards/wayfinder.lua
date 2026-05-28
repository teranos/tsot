return {
  id = "wayfinder",
  symbol = "⋈",
  name = "Wayfinder",
  colors = {"white"},
  type = "creature",
  subtypes = {"human"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 5, source = "mill"},
  },
  abilities = {
    "whenever you draw a card, you may choose to draw from the bottom of your deck.",
  },
  stats = {x = 1, y = 3},
}
