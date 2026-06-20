-- Blue artifact, crystal subtype. 5 mill cost (no hand pitch), and while
-- on BOARD it makes blue spells 1 hand cheaper to cast. A real chemical
-- compound (a thiazine dye that forms striking dark-blue crystals), so
-- the crystal subtype is literal flavor like LCD Clock's.
--
-- Wired via STATIC Phase 3.5 cost_modifiers — affects.kind = "spell"
-- + affects.colors = blue gates the discount to blue instants and
-- sorceries; the engine's play_card pre-pass subtracts 1 from the
-- HAND cost component (clamped to 0 per P.20). Applies to BOTH players
-- (no controller filter) like LCD Clock, since we haven't picked a
-- controller-scoped variant yet for cost reducers.
--
-- The crystal subtype today is largely thematic — 5 mill cost means no
-- HAND payments attach to it on cast, so P.24b crystal-tap can't fire
-- without a separate effect routing attached cards onto it (future
-- shift-style mechanic).
return {
  id = "methylene-blue",
  name = "Methylene Blue",
  colors = {"blue"},
  type = "artifact",
  subtypes = {"crystal"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 5, source = "mill"},
  },
  abilities = {
    "blue cards you cast cost 1 less hand and 2 less graveyard to play.",
  },
  static = {
    affects = {
      colors = {"blue"},
      controller = "owner",
    },
    cost_modifiers = {
      {source = "hand", amount = 1},
      {source = "graveyard", amount = 2},
    },
  },
}
