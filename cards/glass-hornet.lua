-- Yellow glass insect cycle. See glass-dragonfly for the design rationale.
return {
  id = "glass-hornet",
  name = "Glass Hornet",
  colors = {"yellow"},
  type = "creature",
  subtypes = {"insect"},
  cost = {{amount = 3, source = "graveyard"}},
  abilities = {
    "cards can't be attached to this creature.",
    "transparent cards cannot be attached to anything (C.13/C.14 rationale).",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
