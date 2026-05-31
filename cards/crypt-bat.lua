-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "crypt-bat",
  name = "Crypt Bat",
  type = "creature",
  colors = {"black"},
  subtypes = {"bat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 3, y = 1},
  abilities = {"flying."},
}
