-- Red jewel — artifact pitch resource. Sits in hand (can't be cast — the
-- engine auto-enforces this because Artifact isn't routable by play_card),
-- waiting to be attached as a HAND-payment cost to a red card. When
-- attached to a red host, that host gets +1/+1 and (deferred) the
-- activated ability "T: draw a card, discard a card."
--
-- Wired today: the +1/+1 via game.add_modifier from the
-- on_attached_as_cost event.
-- Deferred: the granted Tap-activated ability — needs the activated
-- abilities slice + a static-grant-ability mechanism.
return {
  id = "red-jewel",
  name = "Red Jewel",
  colors = {"red"},
  type = "artifact",
  subtypes = {"jewel"},
  cost = {},
  abilities = {
    "you cannot cast this card.",
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
