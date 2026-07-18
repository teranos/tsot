-- Tap Dance: instant tempo-shuffle. Pay with the `tap` cost source
-- (P.42): tap 2 untapped permanents you control (non-consumptive — they
-- untap at U.2). Effect (A.13): untap target card and tap target card on
-- the BOARD. Either player's BOARD is legal target space for each slot;
-- targets may repeat (untap → tap self is a legal no-op).
--
-- The four colors give the P.42a anchor a wide surface: any tapped
-- permanent sharing purple/blue/cyan/green satisfies the color anchor.
return {
  id = "tap-dance",
  name = "Tap Dance",
  colors = { "purple", "blue", "cyan", "green" },
  type = "instant",
  cost = {
    { amount = 2, source = "tap" },
  },
  abilities = {
    "untap target card and tap target card on the board.",
  },
  on_play = function(game, self)
    -- Both BOARDs, any permanent, are legal target space for each slot.
    local board = {}
    for _, iid in ipairs(game.zones(self.controller).board) do
      table.insert(board, iid)
    end
    for _, iid in ipairs(game.zones(game.opponent(self.controller)).board) do
      table.insert(board, iid)
    end
    if #board == 0 then return end

    local untap_target = game.choose_card(board, { optional = false, prompt = "untap target card" })
    if untap_target then game.untap(untap_target) end

    -- Same card is a legal pick again — untap-then-tap self is a no-op.
    local tap_target = game.choose_card(board, { optional = false, prompt = "tap target card" })
    if tap_target then game.tap(tap_target) end
  end,
}
