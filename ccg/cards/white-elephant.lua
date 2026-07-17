-- White Elephant — a stubborn white creature that will not go quietly.
--
-- The first consumer of the death-replacement hook (Z.8 sleeveless). Its
-- printed rule is a "would die" replacement:
--
--   "If this creature would die: if it is in a sleeve, take it out of the
--    sleeve and attach the sleeve to it and it survives; if it is
--    unsleeved, exile it instead."
--
-- First lethal event — still sleeved: shed_own_sleeve pops the card out of
--   its own sleeve (the emptied sleeve attaches to it, Z.6) and
--   prevent_death saves it — it stays on the BOARD, damage cleared, now
--   sleeveless.
-- Second lethal event — sleeveless: no sleeve left to shed, so
--   redirect_death sends it to EXILE instead of the GRAVEYARD (a quiet
--   relocation — no on_die, no cascade).
return {
  id = "white-elephant",
  name = "White Elephant",
  symbol = "⋈",
  type = "creature",
  colors = {"white"},
  subtypes = {"elephant"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "attach"},
  },
  stats = {x = 4, y = 4},
  abilities = {
    "if this creature would die: if it is in a sleeve, take it out of the sleeve and attach the sleeve to it and it survives; if it is unsleeved, exile it instead.",
  },
  on_would_die = function(game, self)
    if game.is_sleeveless(self.instance_id) then
      -- No sleeve left to shed — exiled instead of dying to the graveyard.
      game.redirect_death(self.instance_id, "exile")
    else
      -- Pop out of its own sleeve; the empty sleeve attaches to it. The
      -- creature survives (damage cleared by the engine on prevent).
      game.shed_own_sleeve(self.instance_id)
      game.prevent_death(self.instance_id)
    end
  end,
  flavor = "It will not go quietly. It will go once.",
}
