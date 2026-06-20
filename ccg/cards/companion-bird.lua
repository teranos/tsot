-- 1/1 bird with intrinsic flying that grants flying to its host when
-- pitched as a HAND-cost attachment. The host-grant uses STATIC Phase 2's
-- AttachedHost scope: while companion-bird is in some on-board host's
-- `attached` list, the static fires with the host as the target and
-- grants `modifier_keyword = "flying"` via GameState::has_keyword.
-- Symbol not yet specified.
return {
  id = "companion-bird",
  name = "Companion Bird",
  type = "creature",
  colors = {"blue", "white"},
  subtypes = {"bird"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "flying.",
    "while this card is attached to a creature, that creature has flying.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {
      scope = "attached_host",
    },
    modifier = {keyword = "flying"},
  },
}
