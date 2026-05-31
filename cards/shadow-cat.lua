-- Vanilla single-color creature for pool diversity.
-- Hand-only cost; 1 baseline, 2 for top-end bodies (3/4, 4/3).
return {
  id = "shadow-cat",
  name = "Shadow Cat",
  type = "creature",
  colors = {"black"},
  subtypes = {"cat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 3, y = 3},
  abilities = {},
}
