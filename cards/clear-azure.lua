-- Transparent artifact with an azure color tag. Same engine role as
-- clear-view (gy_hand_substitute), with the added azure color so that
-- once P.12a lands, exiling this from the graveyard to pay a GRAVEYARD
-- cost component of an azure cast satisfies the color-anchor rule.
return {
  id = "clear-azure",
  name = "Clear Azure",
  colors = {"transparent", "azure"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear azure does not satisfy P.7a identity for the cast — other hand payments must.",
    "may anchor P.12a as an azure GRAVEYARD pitch for an azure cast.",
  },
  gy_hand_substitute = true,
  flavor = "Azure light, no surface.",
}
