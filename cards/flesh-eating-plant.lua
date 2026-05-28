-- Symbol not yet specified.
return {
  id = "flesh-eating-plant",
  name = "Flesh-eating Plant",
  colors = {"red", "green"},
  type = "creature",
  subtypes = {"plant"},
  cost = {{amount = 1, source = "sacrifice"}},
  abilities = {
    "this creature cannot attack.",
    "insects your opponents control cannot attack or be used as a cost paid.",
    "When this creature dies you may return an insect card from your graveyard to your hand.",
  },
  stats = {x = 1, y = 2},
}
