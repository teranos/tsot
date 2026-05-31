-- Red jewel — artifact pitch resource and on-board T-engine. Three modes:
--   1. Pitched from HAND as a payment cost on a red card → attaches to
--      that host and grants +1/+1 (via on_attached_as_cost).
--   2. Cast directly to BOARD for 0 cost (artifacts route through play_card
--      since the artifact-castable change). On BOARD it can be tapped for
--      its own T-ability (mode 3).
--   3. T: draw a card, then discard a card. Smart-discard heuristic picks
--      the least-useful card from hand. Net effect: cycle the worst card
--      for the next-best card. Costs the jewel a turn of being tappable.
--
-- The "host creature gains the jewel's T-ability after attachment" rider
-- noted in earlier comments was deferred indefinitely: static ability
-- grants don't exist as a mechanic yet. Once they do, this card's
-- on_attached_as_cost can apply that grant alongside the +1/+1.
return {
  id = "red-jewel",
  name = "Red Jewel",
  colors = {"red"},
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "T: draw a card, then discard a card.",
    "when this card is attached as a cost to a red card, that creature gets +1/+1.",
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
