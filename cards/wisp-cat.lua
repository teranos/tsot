-- Single-color vanilla cat. Predator-prey override: can block birds
-- despite flying (engine field can_block_subtypes); ability text
-- mirrors the engine rule.
return {
  id = "wisp-cat",
  name = "Wisp Cat",
  type = "creature",
  colors = {"purple"},
  subtypes = {"cat"},
  can_block_subtypes = {"bird"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 1, y = 3},
  abilities = {"can block birds."},
}
