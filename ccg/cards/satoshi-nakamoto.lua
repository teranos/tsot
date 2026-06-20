return {
  id = "satoshi-nakamoto",
  name = "Satoshi Nakamoto",
  colors = {"yellow"},
  type = "creature",
  subtypes = {"human", "legend"},
  symbols = {"₿"},
  cost = {
    {amount = 6, source = "graveyard"},
  },
  abilities = {
    "invulnerability.",
  },
  stats = {x = 1, y = 1},
  variants = {
    -- Probe the 5gy → 6gy step. Both keep invulnerability and the 1/1
    -- body; only the cost moves. Tighter scope = faster probe.
    ["6gy"] = { cost = {{amount = 6, source = "graveyard"}} },
  },
}
