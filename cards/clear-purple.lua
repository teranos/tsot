-- Transparent artifact with a purple color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added purple color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a purple cast satisfies the color-anchor rule.
return {
  id = "clear-purple",
  name = "Clear Purple",
  colors = {"transparent", "purple"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear purple does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a purple GRAVEYARD pitch for a purple cast.",
  },
  gy_hand_substitute = true,
  flavor = "Dusk in clean cut.",
}
