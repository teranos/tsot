-- Colorless artifact: while on BOARD, creatures cost 1 less hand and
-- 1 less graveyard to play. Applies to both players (no controller
-- filter). 10 mill to cast is a real investment — you self-mill 10
-- cards from the top of your deck — but the discount compounds across
-- every creature you cast for the rest of the game.
--
-- Wired via STATIC Phase 3.5 cost-modification layer:
-- `cost_modifiers` carries two entries (hand -1, graveyard -1). The
-- engine's play_card pre-pass reads on-board statics, accumulates
-- per-source reductions, and subtracts from each cost component
-- (clamped to 0 per P.20). `affects.kind = "creature"` gates the
-- discount to creature casts; spells and artifacts pay full price.
--
-- Symbol not yet specified.
return {
  id = "modern-lcd-clock",
  name = "Modern LCD Clock",
  type = "artifact",
  subtypes = {"crystal"},
  cost = {{amount = 5, source = "mill"}},
  abilities = {
    "creatures cost 1 less hand and 1 less graveyard to play. this applies to both players.",
  },
  static = {
    affects = {
      kind = "creature",
    },
    cost_modifiers = {
      {source = "hand", amount = 1},
      {source = "graveyard", amount = 1},
    },
  },
}
