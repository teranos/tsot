-- Red glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-ladybug",
  name = "Glass Ladybug",
  colors = {"red"},
  type = "creature",
  subtypes = {"insect"},
  cost = {
    {amount = 1, source = "graveyard"},
  },
  abilities = {
    "cards can't be attached to this creature.",
  },
  stats = {x = 0.2, y = 0.2},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
