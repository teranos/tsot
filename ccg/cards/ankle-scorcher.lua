-- Ankle Scorcher: red hound with a `tap` cost (P.42) — tap 1 untapped
-- permanent you control (must share red, the P.42a anchor). Comes down
-- with haste (3/1). Its drawback triggers on OnUntapped (the mirror of
-- OnTapped): whenever it becomes untapped — at your untap step, or when
-- something untaps it (e.g. Tap Dance) — you discard a card of your
-- choice. Mandatory when you have a card to discard; you don't get to
-- decline.
--
-- BALANCE NOTE (for EA / probe, not a chat opinion): at tap 1 this may
-- be too strong. The tap cost only needs an untapped permanent, so a
-- second copy can pay by tapping the first — draw multiples in your
-- opening hand and you can chain them out on turn one. The discard is
-- the intended brake, but it only bites at the *next* untap step, after
-- the chained bodies have already landed. Whether that brake is enough
-- is an EA question, not something to reason about from the armchair.
-- The `variants` below let `make probe` measure it directly: hold power
-- fixed and move the tap cost 1->2 (the chain tax), then hold cost fixed
-- and move power 3->2. The base card is the tap-1 / power-3 reference,
-- so `ankle-scorcher` vs `-tap2` isolates cost and `ankle-scorcher` vs
-- `-power2` isolates power.
return {
  id = "ankle-scorcher",
  name = "Ankle Scorcher",
  colors = { "red" },
  type = "creature",
  subtypes = { "hound" },
  cost = {
    { amount = 1, source = "tap" },
  },
  stats = { x = 3, y = 1 },
  abilities = {
    "haste.",
    "whenever this creature becomes untapped, you discard a card.",
  },
  on_untapped = function(game, self)
    local hand = {}
    for _, iid in ipairs(game.zones(self.controller).hand) do
      table.insert(hand, iid)
    end
    if #hand == 0 then return end
    -- Chosen, but mandatory: optional = false means you can't decline
    -- while you have something to discard.
    local target = game.choose_card(hand, { optional = false, prompt = "discard a card" })
    if target then game.move(target, "graveyard") end
  end,
  -- Probe-only (excluded from EA/champions). Override semantics replace
  -- the whole field, so each variant restates the full cost/stats.
  variants = {
    -- Cost axis: same 3/1 body, tap cost 1 -> 2 (taxes the chain).
    ["tap2"] = {
      name = "Ankle Scorcher (tap 2)",
      cost = { { amount = 2, source = "tap" } },
      stats = { x = 3, y = 1 },
    },
    -- Power axis: same tap-1 cost, power 3 -> 2.
    ["power2"] = {
      name = "Ankle Scorcher (2/1)",
      cost = { { amount = 1, source = "tap" } },
      stats = { x = 2, y = 1 },
    },
    -- Fourth corner of the 2x2: expensive body (tap 2) AND small (2/1),
    -- so the grid separates the cost axis from the power axis instead of
    -- only crossing them at the base card.
    ["tap2-power2"] = {
      name = "Ankle Scorcher (tap 2, 2/1)",
      cost = { { amount = 2, source = "tap" } },
      stats = { x = 2, y = 1 },
    },
  },
}
