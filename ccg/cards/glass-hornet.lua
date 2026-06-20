-- Yellow glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-hornet",
  name = "Glass Hornet",
  colors = {"yellow"},
  type = "creature",
  subtypes = {"insect"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "flying.",
    "cards can't be attached to this creature.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
