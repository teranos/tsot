-- Name and symbol not yet specified.
return {
  id = "vigilant-human",
  type = "creature",
  colors = {"white"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}, {amount = 1, source = "graveyard"}},
  abilities = {
    "vigilance.",
    "Tap: if this creature attacked this turn, draw a card.",
  },
  stats = {x = 2, y = 2},
}
