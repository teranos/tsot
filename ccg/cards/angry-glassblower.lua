-- Angry Glassblower — a red human who blows glass sleeves in a temper.
--
-- The second cardless-sleeve card (backlog). Where Window Cleaner pulls
-- clear sleeves out of the DECK, the Glassblower blows a fresh empty
-- sleeve straight out of your HAND and slaps it on himself as he swings;
-- when the swing lands he can shatter one back off for cards.
--
--   On attack: *may* attach an empty sleeve FROM HAND to himself and
--     draw a card. (Uses attach_cardless_from_hand — hand, not deck.)
--   On dealing damage to a player: *may* exile an attached card off
--     himself; if that card was an empty sleeve, draw a card and then
--     discard one — a rummage powered by shattering glass.
--
-- Both triggers are existing events (OnAttack, OnDealtDamageToPlayer) —
-- no OnTapped, no deferred-event queue.
return {
  id = "angry-glassblower",
  name = "Angry Glassblower",
  symbol = "⋈",
  type = "creature",
  colors = {"red"},
  subtypes = {"human"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  stats = {x = 3, y = 4},
  abilities = {
    "when this creature attacks, you may attach an empty sleeve from your hand to it and draw a card.",
    "when this creature deals damage to a player, you may exile an attached card from it; if it was an empty sleeve, draw a card and then discard a card.",
  },
  on_attack = function(game, self)
    -- Only offer the "may" when there is actually an empty sleeve in
    -- hand to blow onto himself.
    local has_empty = false
    for _, iid in ipairs(game.zones(self.owner).hand) do
      if game.is_cardless(iid) then
        has_empty = true
        break
      end
    end
    if not has_empty then return end
    if not game.confirm("Angry Glassblower attacks — attach an empty sleeve from hand and draw?") then
      return
    end
    game.attach_cardless_from_hand(self.instance_id, self.owner, 1)
    game.draw(self.owner, 1)
  end,
  on_dealt_damage_to_player = function(game, self)
    if #self.attached == 0 then return end
    if not game.confirm("Angry Glassblower connected — exile an attached card off him?") then
      return
    end
    local target = game.choose_card(self.attached, {prompt = "exile an attached card"})
    if not target then return end
    local was_empty = game.is_cardless(target)
    game.move(target, "exile")
    if was_empty then
      game.draw(self.owner, 1)
      game.discard(self.owner, 1)
    end
  end,
  flavor = "He'll make you a window. He'll break it too.",
}
