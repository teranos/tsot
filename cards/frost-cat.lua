-- Single-color vanilla cat. Predator-prey override: can block birds
-- despite flying (engine field can_block_subtypes); ability text
-- mirrors the engine rule.
return {
  id = "frost-cat",
  name = "Frost Cat",
  type = "creature",
  colors = {"blue"},
  subtypes = {"cat"},
  can_block_subtypes = {"bird"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 1, y = 1},
  abilities = {"can block birds."},
}
