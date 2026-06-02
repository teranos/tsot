-- Typeless transparent SELF-cost tutor. Two lifecycles:
--   1. Cast: exile per P.5, on_play tutors purple-jewel from deck to hand.
--   2. Pitched as HAND payment for another spell/mutation (C.14 allows
--      transparent here): goes to GRAVEYARD per the spell-payment
--      convention, then later usable via gy_hand_substitute to fill a
--      HAND slot of a future cast.
return {
  id = "clear-purple",
  name = "Clear Purple",
  colors = {"transparent", "purple"},
  cost = {{amount = 1, source = "self"}},
  abilities = {
    "when you play this card, search your deck for a purple-jewel and move it to your hand. self-exile per P.5 — clear purple goes to EXILE on resolution, not GRAVEYARD.",
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. (only reachable when clear purple was previously pitched as HAND payment for another spell.)",
  },
  on_play = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c and c.id == "purple-jewel" then
        game.move(iid, "hand")
        return
      end
    end
  end,
  gy_hand_substitute = true,
  flavor = "Dusk in clean cut.",
}
