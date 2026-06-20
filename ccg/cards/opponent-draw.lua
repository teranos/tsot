-- Black sorcery: literally take the top 2 cards of your opponent's deck
-- into your hand (theft + deck thinning at once). Plus pay a hand discard
-- as the kicker.
--
-- The "draw" terminology is MTG-style theft ("Memory Lapse"-ish). To make
-- this work, the engine grew a cross-player move primitive (game.move_to)
-- that takes a card from wherever it is and places it in another player's
-- specified zone, updating controller to the new player. Owner stays put
-- per RULES T.2 (immutable).
--
-- Net effect:
--   - Opponent loses 2 cards from top of deck (closer to deckout).
--   - Your hand gains 2 cards (real card advantage, not just damage).
--   - You discard 1 card from your hand (cost rider, partial offset).
--
-- Original cost was "1 self" (SelfExile — unsupported). Provisional: 1 hand.
return {
  id = "opponent-draw",
  name = "Opponent Draw",
  symbol = "⨳",
  colors = {"black"},
  type = "sorcery",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "draw two cards from your opponents deck, discard a card.",
  },
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    for _ = 1, 2 do
      local top = game.deck_top(opp)
      if top then
        game.move_to(top, self.owner, "hand")
      end
    end
    game.discard(self.owner, 1)
  end,
}
