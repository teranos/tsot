-- Vanilla single-color creature for pool diversity.
-- Hand-only cost; 1 baseline, 2 for top-end bodies (3/4, 4/3).
return {
  id = "rune-rat",
  name = "Rune Rat",
  type = "creature",
  colors = {"purple"},
  subtypes = {"rat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 3, y = 3},
  abilities = {},
}
