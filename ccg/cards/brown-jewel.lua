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
  on_zone_change = function(game, self, moving, from, to)
    if moving.instance_id ~= self.instance_id then return end
    if to ~= "board" then return end
    local host_iid = game.host_of(self.instance_id)
    if host_iid then
      if from ~= "hand" then return end
      local p = game.card(host_iid)
      if not p or not p.colors then return end
      for _, col in ipairs(p.colors) do
        if col == "brown" then
          game.add_modifier(host_iid, "stat_boost", 1, 1)
          return
        end
      end
    else
      game.tap(self.instance_id)
    end
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
