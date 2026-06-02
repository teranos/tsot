-- Transparent artifact with an orange color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added orange color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of an orange cast satisfies the color-anchor rule.
return {
  id = "clear-orange",
  name = "Clear Orange",
  colors = {"transparent", "orange"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear orange does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as an orange GRAVEYARD pitch for an orange cast.",
  },
  gy_hand_substitute = true,
  flavor = "Glass at sundown.",
}
