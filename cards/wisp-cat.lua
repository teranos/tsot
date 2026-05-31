-- Vanilla single-color creature for pool diversity.
-- Hand-only cost; 1 baseline, 2 for top-end bodies (3/4, 4/3).
return {
  id = "wisp-cat",
  name = "Wisp Cat",
  type = "creature",
  colors = {"purple"},
  subtypes = {"cat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 1, y = 3},
  abilities = {},
}
