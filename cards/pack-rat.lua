-- Vanilla single-color creature for pool diversity.
-- Hand-only cost; 1 baseline, 2 for top-end bodies (3/4, 4/3).
return {
  id = "pack-rat",
  name = "Pack Rat",
  type = "creature",
  colors = {"green"},
  subtypes = {"rat"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 4, y = 3},
  abilities = {},
}
