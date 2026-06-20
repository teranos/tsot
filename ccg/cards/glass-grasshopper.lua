-- Green glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-grasshopper",
  name = "Glass Grasshopper",
  colors = {"green"},
  type = "creature",
  subtypes = {"insect"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  abilities = {
    "cards can't be attached to this creature.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
