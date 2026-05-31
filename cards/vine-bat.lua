-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "vine-bat",
  name = "Vine Bat",
  type = "creature",
  colors = {"green"},
  subtypes = {"bat"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 3, y = 4},
  abilities = {"flying."},
}
