-- Transparent artifact with a red color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added red color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of a red cast satisfies the color-anchor rule.
return {
  id = "clear-red",
  name = "Clear Red",
  colors = {"transparent", "red"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear red does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as a red GRAVEYARD pitch for a red cast.",
  },
  gy_hand_substitute = true,
  flavor = "Translucent burn.",
}
