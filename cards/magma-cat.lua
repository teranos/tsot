-- Single-color vanilla cat. Predator-prey override: can block birds
-- despite flying (engine field can_block_subtypes); ability text
-- mirrors the engine rule.
return {
  id = "magma-cat",
  name = "Magma Cat",
  type = "creature",
  colors = {"red"},
  subtypes = {"cat"},
  can_block_subtypes = {"bird"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 3, y = 4},
  abilities = {"can block birds."},
}
