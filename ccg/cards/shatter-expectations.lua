-- Shatter Expectations — a colourless instant that counters a spell
-- unless its controller ransoms it across every zone.
--
-- Slice 10. Three novel pieces, all resolved in on_play (Shatter is a
-- free instant that responds to a cast, target = chain):
--
--   1. Composition-derived X. You exile any number of cards from your
--      own graveyard. X is NOT chosen — it is the net composition of
--      what you exiled: each CLEAR or CARDLESS sleeve adds 1, each
--      ordinary card subtracts 1. Pure clear/empty maximises X; padding
--      it with ordinary cards only weakens the threat.
--
--   2. Counter-with-alternative-cost. Counter the target spell UNLESS
--      its controller chooses to pay the ransom. The prompt goes to the
--      OPPONENT via game.confirm_for — an opponent-side may-pay.
--
--   3. Multi-zone exile. The ransom is X from HAND, X from GRAVEYARD, X
--      from BOARD, and X from DECK — 4X cards across four zones at once.
--
-- Non-positive X (a net-negative or all-ordinary payment) can't threaten
-- anything: the ransom is trivially met, so the spell simply resolves.
--
-- Flavour: "he paid it!?"
--
-- Entire top and bottom rows are transparent slots (holes).
local function exile_top_n(game, pid, zone, n)
  for _ = 1, n do
    local ids = game.zones(pid)[zone]
    if #ids == 0 then break end
    game.move(ids[1], "exile")
  end
end

return {
  id = "shatter-expectations",
  name = "Shatter Expectations",
  type = "instant",
  colors = {},
  cost = {},
  target = "chain",
  holes = {"TL", "T", "TR", "BL", "B", "BR"},
  abilities = {
    "exile any number of cards from your graveyard. X = (clears + cardless sleeves exiled) − (other cards exiled).",
    "counter target spell, unless its controller exiles X from their hand, X from their graveyard, X from their board, and X from their deck.",
  },
  on_play = function(game, self)
    local me = self.owner

    -- 1. Composition-derived X: caster exiles chosen graveyard cards.
    local x = 0
    while true do
      local gy = game.zones(me).graveyard
      if #gy == 0 then break end
      local pick = game.choose_card_for(me, gy, {
        optional = true,
        prompt = "exile a card from your graveyard to Shatter (cancel to stop)",
      })
      if not pick then break end
      if game.is_clear(pick) or game.is_cardless(pick) then
        x = x + 1
      else
        x = x - 1
      end
      game.move(pick, "exile")
    end

    -- 2. A non-positive X threatens nothing — the spell resolves.
    if x <= 0 then return end

    -- 3. Counter target spell unless its controller pays the 4X ransom.
    local opp = game.opponent(me)
    local z = game.zones(opp)
    local can_pay = #z.hand >= x and #z.graveyard >= x and #z.board >= x and #z.deck >= x
    local pays = false
    if can_pay then
      pays = game.confirm_for(
        opp,
        "Shatter Expectations: exile " .. x .. " from HAND, " .. x ..
          " from GRAVEYARD, " .. x .. " from BOARD, and " .. x ..
          " from DECK to save your spell?"
      )
    end

    if pays then
      exile_top_n(game, opp, "hand", x)
      exile_top_n(game, opp, "graveyard", x)
      exile_top_n(game, opp, "board", x)
      exile_top_n(game, opp, "deck", x)
      -- Ransom paid — the spell is NOT countered; it resolves.
    else
      game.counter_top()
    end
  end,
  flavor = "he paid it!?",
}
