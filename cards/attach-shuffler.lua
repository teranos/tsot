-- Name and symbol not yet specified.
return {
  id = "attach-shuffler",
  type = "creature",
  colors = {"green"},
  subtypes = {"human"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "whenever this creature attacks you may attach a card and return another attached card you own back to your hand.",
    "whenever a creature dies because it blocked this creature, return this creature to your hand at the end of the turn.",
  },
  stats = {x = 3, y = 2},
}
