-- Red mutation in the protein-gene cycle (Klotho/FST). Free to cast.
-- Grants the host creature +2/+1 via an `attached_host`-scope static.
-- Wired today (no missing-event dependency like klotho has) because
-- the effect is a continuous stat modifier, not a triggered ability.
return {
  id = "FST",
  name = "FST",
  type = "mutation",
  colors = {"red"},
  cost = {},
  abilities = {
    "the host creature gets +2/+1.",
  },
  static = {
    affects = {scope = "attached_host"},
    modifier = {x = 2, y = 1},
  },
}
