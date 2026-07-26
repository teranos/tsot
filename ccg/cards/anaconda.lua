-- Green/purple 2/2 snake — third member of the reveal-on-attach cycle
-- (mantis-shrimp = red/blue, zebra = black/white). If anaconda gets
-- attached as a HAND-payment cost to a green OR purple card, may reveal
-- it and draw a card. Same OnAttachedAsCost handler shape: read the
-- host card's colors via game.card(partner.instance_id), match against
-- own color identity, gate the draw on game.confirm.
--
-- Completes the cycle's color coverage of the 6-color jewel system:
-- shrimp covers R/U, zebra covers B/W, anaconda covers G/Pu. Together
-- the three turn pitched-into-color into a "free draw" subsidy for
-- every color in the game.
return {
  id = "anaconda",
  name = "Anaconda",
  symbol = "꩜",
  colors = {"green", "purple"},
  type = "creature",
  subtypes = {"snake"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "If this card gets attached as a cost to a green or purple card, you may reveal it and draw a card.",
  },
  stats = {x = 2, y = 2},
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    local matches = false
    for _, col in ipairs(p.colors) do
      if col == "green" or col == "purple" then
        matches = true
        break
      end
    end
    if not matches then return end
    if not game.confirm("reveal anaconda to draw a card?") then return end
    game.draw(self.owner, 1)
  end,
}
