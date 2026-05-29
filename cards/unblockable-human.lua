-- Name and symbol not yet specified.
return {
  id = "unblockable-human",
  type = "creature",
  colors = {"blue"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "unblockable.",
    "when this creature attacks you may exile a card from your graveyard; if you do, this creature gets +2/+0 until end of turn.",
    "when this creature attacks a player you may discard a card and draw a card.",
  },
  stats = {x = 0, y = 1},
}
