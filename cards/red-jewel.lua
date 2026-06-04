-- Red jewel — artifact pitch resource and on-board T-engine. Three modes:
--   1. Pitched from HAND as a payment cost on a red card → attaches to
--      that host and grants +1/+1 (via on_attached_as_cost). The
--      attached static then grants `T: draw a card, then discard` to
--      the host creature (Phase 3 static-granted activations).
--   2. Cast directly to BOARD for 0 cost. On BOARD it can be tapped
--      for its own T-ability (its printed `activated`).
--   3. T: draw a card, then discard a card. Whether fired from the
--      jewel on board or from a host that has the jewel attached, the
--      effect is identical: 1 card drawn, 1 card discarded.
return {
  id = "red-jewel",
  name = "Red Jewel",
  colors = { C = "red" },
  symbol = "⨳",
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "T: pay for one hand-source component of a card you cast that shares a color with this jewel.",
    "T: draw a card, then discard a card.",
    "when this card is attached as a cost to a red card, that creature gets +1/+1 and gains: T: draw a card, then discard a card.",
  },
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    for _, col in ipairs(p.colors) do
      if col == "red" then
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
