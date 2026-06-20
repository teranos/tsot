return {
  id = "hollow",
  name = "Hollow",
  type = "creature",
  cost = {
    {amount = 1, source = "attached"},
    {amount = 2, source = "mill"},
  },
  stats = {x = 0, y = 0},
  abilities = {
    "this creature gets +1/+1 for each attached card.",
  },
  static = {
    affects = {scope = "source_only"},
    modifier = {x = "attached", y = "attached"},
  },
}
