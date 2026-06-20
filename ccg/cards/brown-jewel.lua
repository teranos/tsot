-- Brown jewel — 9th member of the jewel cycle. Same template as the
-- other eight: free cost, T-engine, OnAttachedAsCost grants +1/+1 to
-- a matching-color host. See red-jewel for the cycle design rationale.
return {
  id = "brown-jewel",
  name = "Brown Jewel",
  colors = { C = "brown" },
  symbol = "⨳",
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "T: pay for one hand-source component of a card you cast that shares a color with this jewel.",
    "T: draw a card, then discard a card.",
    "when this card is attached as a cost to a brown card, that creature gets +1/+1.",
  },
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    for _, col in ipairs(p.colors) do
      if col == "brown" then
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
}
