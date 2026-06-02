-- Transparent artifact with a blue color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added blue color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a blue cast satisfies the color-anchor rule.
return {
  id = "clear-blue",
  name = "Clear Blue",
  colors = {"transparent", "blue"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear blue does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a blue GRAVEYARD pitch for a blue cast.",
  },
  gy_hand_substitute = true,
  flavor = "A clean idea.",
}
