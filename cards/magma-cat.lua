-- Vanilla single-color creature for pool diversity.
-- Hand-only cost; 1 baseline, 2 for top-end bodies (3/4, 4/3).
return {
  id = "magma-cat",
  name = "Magma Cat",
  type = "creature",
  colors = {"red"},
  subtypes = {"cat"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 3, y = 4},
  abilities = {},
}
