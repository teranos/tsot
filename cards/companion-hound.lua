-- 1/1 hound with intrinsic vigilance that grants vigilance to its host
-- when pitched as a HAND-cost attachment. Same AttachedHost-scope
-- static pattern as companion-bird (flying) and companion-hare (haste).
return {
  id = "companion-hound",
  name = "Companion Hound",
  type = "creature",
  colors = {"white"},
  subtypes = {"hound"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "vigilance.",
    "while this card is attached to a creature, that creature has vigilance.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {
      scope = "attached_host",
    },
    modifier = {keyword = "vigilance"},
  },
}
