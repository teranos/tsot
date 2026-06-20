-- Green hydra. Variable-X cost: X hand cards attach as payment. Stats
-- scale +1/+1 per attached card and STAY accurate as the attached set
-- changes — wired via STATIC Phase 1.5 dynamic modifier (ModifierValue::
-- AttachedCount). If falter strips the attached cards later, hydra's
-- stats shrink with them. Same shape works for any future "X/Y per
-- attached <thing>" creature.
--
-- The `2pwr` variant scales +2/+2 per attached via the
-- `ModifierValue::AttachedCountScaled(2)` form (string `"2*attached"`).
--
-- Symbol not yet specified.
return {
  id = "hydra",
  colors = {"green"},
  type = "creature",
  subtypes = {"hydra"},
  cost = {{is_x = true, source = "hand"}},
  abilities = {
    "this creature gets +1/+1 for each attached card.",
  },
  stats = {x = 0, y = 0},
  static = {
    affects = {
      scope = "source_only",
    },
    modifier = {x = "attached", y = "attached"},
  },
  variants = {
    ["2pwr"] = {
      abilities = {
        "this creature gets +2/+2 for each attached card.",
      },
      static = {
        affects = { scope = "source_only" },
        modifier = {x = "2*attached", y = "2*attached"},
      },
    },
  },
}
