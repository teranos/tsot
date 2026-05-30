-- All-color artifact: 1 hand, subtype "crystal", a variant of the jewel
-- tap-payment mechanic (P.24) but with a twist on color matching.
--
-- Like jewels, this card can be tapped on the BOARD to substitute for one
-- HAND-source cost component of a card being cast. UNLIKE jewels — whose
-- own color must match the cast card — the crystal's own colors are all
-- six and would always match trivially. So the engine instead checks the
-- colors of cards ATTACHED to the crystal: at least one attached card
-- must share a color with the cast card.
--
-- This makes the crystal cold the moment it lands (no attached colors yet),
-- but it picks up power as you pitch cards to it as HAND payment when you
-- recast or refurbish. Played alone it's a placeholder; played after
-- cards have been attached to other artifacts (via the existing attach
-- routing), it acquires the colors of those attachments.
--
-- Wait — attached cards live on the played card, not the crystal. The
-- crystal needs cards attached TO IT. With cost = 1 hand, when you cast
-- the crystal, the 1 hand payment attaches to the crystal. So the first
-- attached card is whatever you paid the 1 hand cost with. The crystal's
-- effective matching domain starts with that one card's color, and grows
-- if other effects attach more cards to it.
--
-- Symbol not yet specified.
return {
  id = "crystal-shard-of-potential",
  name = "Crystal Shard of Potential",
  colors = {"black", "blue", "green", "purple", "red", "white"},
  type = "artifact",
  subtypes = {"crystal"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "T: pay for one hand-source component of a card you cast. The attached card's color must match the cast card.",
  },
}
