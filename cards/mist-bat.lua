-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "mist-bat",
  name = "Mist Bat",
  type = "creature",
  colors = {"blue"},
  subtypes = {"bat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 1, y = 3},
  abilities = {"flying."},
}
