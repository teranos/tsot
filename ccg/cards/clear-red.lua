-- Typeless transparent SELF-cost tutor. See clear-purple for the
-- two-lifecycle design rationale.
return {
  id = "clear-red",
  name = "Clear Red",
  colors = {"red"},
  frame = "transparent",
  cost = {{amount = 1, source = "self"}},
  abilities = {
    "when you play this card, search your deck for a red-jewel OR a red-symbol card and move it to your hand. self-exile per P.5 — clear red goes to EXILE on resolution, not GRAVEYARD.",
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast.",
  },
  on_play = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c then
        if c.id == "red-jewel" then
          game.move(iid, "hand")
          return
        end
        if c.type == "symbol" and c.colors then
          for _, col in ipairs(c.colors) do
            if col == "red" then
              game.move(iid, "hand")
              return
            end
          end
        end
      end
    end
  end,
  gy_hand_substitute = true,
  flavor = "Translucent burn.",
}
