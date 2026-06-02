-- Typeless transparent SELF-cost tutor. Cast it, exile it (per P.5),
-- on resolution it tutors green-jewel from deck to hand.
return {
  id = "clear-green",
  name = "Clear Green",
  colors = {"green"},
  frame = "transparent",
  cost = {{amount = 1, source = "self"}},
  abilities = {
    "when you play this card, search your deck for a green-jewel and move it to your hand. self-exile per P.5 — clear green goes to EXILE on resolution, not GRAVEYARD.",
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast.",
  },
  on_play = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c and c.id == "green-jewel" then
        game.move(iid, "hand")
        return
      end
    end
  end,
  gy_hand_substitute = true,
  flavor = "Looks through leaves.",
}
