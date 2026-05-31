-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "witch-bat",
  name = "Witch Bat",
  type = "creature",
  colors = {"purple"},
  subtypes = {"bat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 3, y = 2},
  abilities = {"flying."},
}
