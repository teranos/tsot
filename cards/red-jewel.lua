-- Red jewel — artifact pitch resource. Two modes:
--   1. Pitched from HAND as a payment cost on a red card → attaches to
--      that host and grants +1/+1 (via on_attached_as_cost).
--   2. Cast directly to BOARD for 0 cost (artifacts route through play_card
--      since the artifact-castable change). On BOARD it's currently inert
--      until the jewel-tap-as-cost mechanic lands.
--
-- The granted "T: draw a card, discard a card" is deferred until
-- activated abilities + static-grant-ability land.
return {
  id = "red-jewel",
  name = "Red Jewel",
  colors = {"red"},
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "when this card is attached as a cost to a red card, that creature gets +1/+1 and gains: T: draw a card, discard a card.",
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
}
