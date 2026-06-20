-- 1/1 hare with intrinsic haste that grants haste to its host when
-- pitched as a HAND-cost attachment. Mirrors companion-bird's pattern
-- with STATIC Phase 2's AttachedHost scope: while companion-hare is in
-- an on-board host's `attached` list, the static fires with the host
-- as the target and grants `modifier_keyword = "haste"`.
return {
  id = "companion-hare",
  name = "Companion Hare",
  type = "creature",
  colors = {"red"},
  subtypes = {"hare"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "haste.",
    "while this card is attached to a creature, that creature has haste.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {
      scope = "attached_host",
    },
    modifier = {keyword = "haste"},
  },
}
