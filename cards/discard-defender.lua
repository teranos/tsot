-- Name and symbol not yet specified.
return {
  id = "discard-defender",
  type = "creature",
  colors = {"black"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "defender.",
    "whenever this creature blocks, you may discard X cards and draw a card (X can be 0).",
    "whenever you discard a card, this creature gets +1/+0.",
  },
  stats = {x = 1, y = 3},
}
