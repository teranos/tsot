-- Spirit Wanderer: a black/purple spirit that only comes back from the
-- dead. `cast_zones = {"graveyard"}` makes it graveyard-only (P.41b): it
-- is inert in hand and can only be cast from the GRAVEYARD (P.41a). Its
-- cost is 1 graveyard + 3 tap; because the card leaves the graveyard at
-- announcement (P.41c) the `1 gy` must be paid by ANOTHER graveyard card,
-- and the P.42a color anchor can be supplied by any color-matching
-- payment (a black/purple tapped permanent, or the graveyard pitch).
-- Being a creature, it enters the BOARD when cast (P.41d leaves board
-- destinations unchanged), and comes down with haste. Dies, returns to
-- the graveyard, and can be cast again — the wanderer that won't stay
-- buried.
return {
  id = "spirit-wanderer",
  name = "Spirit Wanderer",
  colors = { "black", "purple" },
  type = "creature",
  subtypes = { "spirit" },
  cast_zones = { "graveyard" },
  cost = {
    { amount = 1, source = "graveyard" },
    { amount = 3, source = "tap" },
  },
  stats = { x = 3, y = 1 },
  abilities = {
    "you may only cast this card from the graveyard.",
    "haste.",
  },
}
