-- Azure jewel — 8th member of the jewel cycle. Same template as the
-- other seven: free cost, T-engine, OnAttachedAsCost grants +1/+1 to
-- a matching-color host. See red-jewel for the cycle design rationale.
return {
  id = "azure-jewel",
  name = "Azure Jewel",
  colors = { C = "azure" },
  symbol = "⨳",
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "T: pay for one hand-source component of a card you cast that shares a color with this jewel.",
    "T: draw a card, then discard a card.",
    "when this card is attached as a cost to an azure card, that creature gets +1/+1.",
  },
  on_zone_change = function(game, self, moving, from, to)
    if moving.instance_id ~= self.instance_id then return end
    if to ~= "board" then return end
    local host_iid = game.host_of(self.instance_id)
    if host_iid then
      -- Attached as HAND payment (P.6): +1/+1 on matching-color host.
      if from ~= "hand" then return end
      local p = game.card(host_iid)
      if not p or not p.colors then return end
      for _, col in ipairs(p.colors) do
        if col == "azure" then
          game.add_modifier(host_iid, "stat_boost", 1, 1)
          return
        end
      end
    else
      -- ETB proper: enters tapped.
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
