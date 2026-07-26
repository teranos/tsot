-- Red/blue 2/2 with a pitch-synergy cantrip: if this card was attached as
-- a HAND-payment cost to a red OR blue card, may reveal it and draw a
-- card. Implemented via the new OnAttachedAsCost event — the handler
-- fires the moment this card gets attached to a host, sees the host as
-- `partner`, and checks partner.colors.
--
-- Companion to zebra (black/white). Both make their colors of cards
-- cheaper to play by giving a free draw when used as cost — a "you would
-- have paid 1 anyway, now it's a 1 hand draw-1" play pattern.
return {
  id = "mantis-shrimp",
  name = "Mantis Shrimp",
  colors = {"red", "blue"},
  type = "creature",
  subtypes = {"shrimp"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "If this card gets attached as a cost to a red or blue card, you may reveal it and draw a card.",
  },
  stats = {x = 2, y = 2},
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    local matches = false
    for _, col in ipairs(p.colors) do
      if col == "red" or col == "blue" then
        matches = true
        break
      end
    end
    if not matches then return end
    if not game.confirm("reveal mantis shrimp to draw a card?") then return end
    game.draw(self.owner, 1)
  end,
}
