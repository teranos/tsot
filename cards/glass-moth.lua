-- Pink glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-moth",
  name = "Glass Moth",
  colors = {"pink"},
  type = "creature",
  subtypes = {"insect"},
  cost = {
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "flying.",
    "cards can't be attached to this creature.",
  },
  stats = {x = 0.8, y = 0.8},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
