-- Single-color flying creature. Bats fly; rats and cats don't.
return {
  id = "ember-bat",
  name = "Ember Bat",
  type = "creature",
  colors = {"red"},
  subtypes = {"bat"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 2, y = 3},
  abilities = {"flying."},
}
