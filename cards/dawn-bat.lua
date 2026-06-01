-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "dawn-bat",
  name = "Dawn Bat",
  type = "creature",
  colors = {"white"},
  subtypes = {"bat"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 3, y = 0},
  abilities = {"flying."},
}
