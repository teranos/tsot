-- Single-color vanilla cat. Predator-prey override: can block birds
-- despite flying (engine field can_block_subtypes); ability text
-- mirrors the engine rule.
return {
  id = "shadow-cat",
  name = "Shadow Cat",
  type = "creature",
  colors = {"black"},
  subtypes = {"cat"},
  can_block_subtypes = {"bird"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 2, y = 2},
  abilities = {"can block birds."},
}
