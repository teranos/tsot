-- Transparent artifact with a pink color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added pink color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a pink cast satisfies the color-anchor rule.
return {
  id = "clear-pink",
  name = "Clear Pink",
  colors = {"transparent", "pink"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear pink does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a pink GRAVEYARD pitch for a pink cast.",
  },
  gy_hand_substitute = true,
  flavor = "Memory of a flush.",
}
