-- White sorcery: 2 hand. Both players draw 2, then you exile 2 of their
-- cards from their hand AFTER the draw (so the cards they just drew are
-- eligible targets). Net hand-count change:
--   You:  -2 (cost) -1 (spell leaves hand) +2 (draw) = -1
--   Opp:  +2 (draw) -2 (exile) = 0
-- But opp loses choice over which 2 cards they keep — strict information /
-- selection win for the caster.
--
-- Edge case: if opp's deck is too short for 2, game.draw triggers L.1 and
-- they lose on the spot. Hidden mill finisher against a near-decked opp.
--
-- The "look at" semantic is free — handlers see all card data via
-- game.card(iid). For a human UI this becomes a reveal step; today the
-- AI already has the info.
--
-- AI target selection: choose_card here passes asker = self.owner and a
-- pool entirely of opp-controlled cards. The prefer-opp heuristic fires
-- but every candidate qualifies, so it falls back to roughly arbitrary
-- picks. A smarter target heuristic could pick highest-impact cards.
--
-- Symbol not yet specified.
return {
  id = "glaring-sunlight",
  name = "Glaring Sunlight",
  type = "sorcery",
  colors = {"white"},
  cost = {{amount = 2, source = "hand"}},
  abilities = {
    "each player draws 2 cards. then you exile 2 cards from your opponent's hand.",
  },
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    game.draw(self.owner, 2)
    game.draw(opp, 2)
    -- After both draws, exile up to 2 from opp's hand.
    for i = 1, 2 do
      local pool = {}
      for _, iid in ipairs(game.zones(opp).hand) do
        table.insert(pool, iid)
      end
      if #pool == 0 then return end
      local target = game.choose_card(pool, {prompt = "exile from opponent's hand (" .. i .. "/2)"})
      if not target then return end
      game.move(target, "exile")
    end
  end,
}
