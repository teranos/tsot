-- Black glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-ant",
  name = "Glass Ant",
  colors = {"black"},
  holes = {"TR", "T", "U", "UR", "R"},
  type = "creature",
  subtypes = {"insect"},
  cost = {
    {amount = 1, source = "graveyard"},
  },
  abilities = {
    "cards can't be attached to this creature.",
  },
  stats = {x = 0.1, y = 0.1},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
