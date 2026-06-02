-- Transparent artifact with a white color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added white color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a white cast satisfies the color-anchor rule.
return {
  id = "clear-white",
  name = "Clear White",
  colors = {"transparent", "white"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear white does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a white GRAVEYARD pitch for a white cast.",
  },
  gy_hand_substitute = true,
  flavor = "Empty page, signed.",
}
