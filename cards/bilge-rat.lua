-- Vanilla single-color creature for pool diversity.
-- Hand-only cost; 1 baseline, 2 for top-end bodies (3/4, 4/3).
return {
  id = "bilge-rat",
  name = "Bilge Rat",
  type = "creature",
  colors = {"blue"},
  subtypes = {"rat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 2, y = 3},
  abilities = {},
}
