-- Black/white 2/2 with a pitch-synergy cantrip — the black/white twin of
-- mantis-shrimp. Wired via the OnAttachedAsCost event: when this card is
-- attached as a HAND-payment cost to a black or white host, may reveal
-- it and draw a card.
return {
  id = "zebra",
  symbol = "⨳",
  colors = {"black", "white"},
  type = "creature",
  subtypes = {"Zebra"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "If this card gets attached as a cost to a black or white card, you may reveal it and draw a card.",
  },
  stats = {x = 2, y = 2},
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    local matches = false
    for _, col in ipairs(p.colors) do
      if col == "black" or col == "white" then
        matches = true
        break
      end
    end
    if not matches then return end
    if not game.confirm("reveal zebra to draw a card?") then return end
    game.draw(self.owner, 1)
  end,
}
