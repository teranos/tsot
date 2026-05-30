-- White jewel — artifact pitch resource. See red-jewel for the cycle
-- design rationale.
return {
  id = "white-jewel",
  name = "White Jewel",
  colors = {"white"},
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "you cannot cast this card.",
    "when this card is attached as a cost to a white card, that creature gets +1/+1 and gains: T: draw a card, discard a card.",
  },
  on_attached_as_cost = function(game, self, partner)
    local p = game.card(partner.instance_id)
    if not p or not p.colors then return end
    for _, col in ipairs(p.colors) do
      if col == "white" then
        game.add_modifier(partner.instance_id, "stat_boost", 1, 1)
        return
      end
    end
  end,
}
