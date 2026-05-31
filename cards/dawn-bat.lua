-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "dawn-bat",
  name = "Dawn Bat",
  type = "creature",
  colors = {"white"},
  subtypes = {"bat"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 4, y = 3},
  abilities = {"flying."},
}
