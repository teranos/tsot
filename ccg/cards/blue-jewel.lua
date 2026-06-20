-- Blue jewel — artifact pitch resource and on-board T-engine. See
-- red-jewel for the cycle design rationale. The attached-host static
-- grants `T: draw, discard` to creatures the jewel is pitched onto
-- (Phase 3 static-granted activations).
return {
  id = "blue-jewel",
  name = "Blue Jewel",
  colors = { C = "blue" },
  symbol = "⨳",
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "T: pay for one hand-source component of a card you cast that shares a color with this jewel.",
    "T: draw a card, then discard a card.",
    "when this card is attached as a cost to a blue card, that creature gets +1/+1 and gains: T: draw a card, then discard a card.",
  },
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    for _, col in ipairs(p.colors) do
      if col == "blue" then
        game.add_modifier(partner.instance_id, "stat_boost", 1, 1)
        return
      end
    end
  end,
  on_enter_board = function(game, self)
    game.tap(self.instance_id)
  end,
  activated = {
    {
      cost = "tap",
      text = "T: draw a card, then discard a card.",
      timing = "instant",
      effect = function(game, self)
        game.draw(self.owner, 1)
        game.discard(self.owner, 1)
      end,
    },
  },
  static = {
    affects = {
      scope = "attached_host",
    },
    granted_activated = {
      cost = "tap",
      text = "T: draw a card, then discard a card.",
      timing = "instant",
      effect = function(game, self)
        game.draw(self.owner, 1)
        game.discard(self.owner, 1)
      end,
    },
  },
}
