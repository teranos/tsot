-- Typeless transparent SELF-cost tutor. Cast it, exile it (per P.5),
-- and on resolution it pulls the matching azure-jewel from your deck
-- to your hand. The tutor-as-tax design means you can't double-dip
-- gy_hand_substitute on the same instance — exiling consumes the card.
return {
  id = "clear-azure",
  name = "Clear Azure",
  colors = {"azure"},
  frame = "transparent",
  cost = {{amount = 1, source = "self"}},
  abilities = {
    "when you play this card, search your deck for an azure-jewel and move it to your hand. self-exile per P.5 — clear azure goes to EXILE on resolution, not GRAVEYARD.",
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast.",
  },
  on_play = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c then
        if c.id == "azure-jewel" then
          game.move(iid, "hand")
          return
        end
        if c.type == "symbol" and c.colors then
          for _, col in ipairs(c.colors) do
            if col == "azure" then
              game.move(iid, "hand")
              return
            end
          end
        end
      end
    end
  end,
  gy_hand_substitute = true,
  flavor = "Azure light, no surface.",
}
