-- Green stomper. "Unfair" because it ignores two normal combat rules:
-- it can defend on multiple fronts at once, and it doesn't tap to attack.
--
-- Engine support:
--   - vigilance: enforced today via has_keyword("vigilance") in declare_attacker.
--   - multi-block ("can block more than one creature"): the engine does not
--     currently enforce 1-to-1 blocker assignment — every creature can already
--     block multiple attackers because declare_blocker only checks per-attacker
--     uniqueness, not cross-attacker. The keyword is forward-looking: when the
--     cross-attacker enforcement lands as part of combat tightening, this card
--     will be exempted via has_keyword("multi-block").
--
-- Cost: 2 hand attached + 3 graveyard exiled + 2 from deck top (encoded as
-- mill per the existing CostSource vocabulary; mill = top of deck → graveyard).
-- Symbol not yet specified.
return {
  id = "unfair-stomper",
  name = "Unfair Stomper",
  colors = {"green"},
  type = "creature",
  subtypes = {"beast"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 3, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "vigilance.",
    "multi-block.",
    "this creature can block more than one creature.",
    "this creature does not have to tap in order to attack.",
  },
  stats = {x = 3, y = 4},
}
