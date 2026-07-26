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
  on_zone_change = function(game, self, moving, from, to)
    if moving.instance_id ~= self.instance_id then return end
    if from ~= "hand" or to ~= "board" then return end
    local host_iid = game.host_of(self.instance_id)
    if not host_iid then return end
    local p = game.card(host_iid)
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
