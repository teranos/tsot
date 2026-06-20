-- White glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-mantis",
  name = "Glass Mantis",
  colors = {"white"},
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
