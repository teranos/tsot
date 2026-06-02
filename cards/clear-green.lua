-- Transparent artifact with a green color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added green color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a green cast satisfies the color-anchor rule.
return {
  id = "clear-green",
  name = "Clear Green",
  colors = {"transparent", "green"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear green does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a green GRAVEYARD pitch for a green cast.",
  },
  gy_hand_substitute = true,
  flavor = "Looks through leaves.",
}
