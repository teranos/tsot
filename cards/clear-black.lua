-- Transparent artifact with a black color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added black color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a black cast satisfies the color-anchor rule.
return {
  id = "clear-black",
  name = "Clear Black",
  colors = {"transparent", "black"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear black does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a black GRAVEYARD pitch for a black cast.",
  },
  gy_hand_substitute = true,
  flavor = "Visible only where it isn't.",
}
