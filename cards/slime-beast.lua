-- Pink slime beast — 3/3 defender body that "wilts" when it commits to
-- blocking. The on_block trigger applies -2/-0 UEOT, so a 3/3 blocker
-- effectively swings combat math as a 1/3 the turn it blocks. Strong
-- soak (3 toughness absorbs almost any attacker), weak retaliation
-- (only 1 power into the attacker).
--
-- Color-fits-mechanic: pink is the "soft answers" color (see EA design
-- discussion). Slime Beast doesn't trade hard — it deflects, dampens,
-- survives. Blocking is pink's contribution to combat: "I'll stop the
-- attack but I'm not here to kill you back."
--
-- Cost: 1 hand + 2 graveyard. Graveyard-heavy gates it past the early
-- turns when the GY is empty, fitting pink's mid-game restorative
-- identity. A 3/3 body for 1 hand is generous; the -2/-0 on block is
-- the balancer that prices the defensive value.
--
-- Mechanically: the EOT modifier uses the standard EotStatBoost variant
-- (negative deltas allowed), same machinery unblockable-human's +2/+0
-- pump and bring-down's -3/-3 removal both ride on.
return {
  id = "slime-beast",
  name = "Slime Beast",
  symbol = "꩜",
  colors = {"pink"},
  type = "creature",
  subtypes = {"slime", "beast"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "When this creature blocks, it gets -2/-0 until end of turn.",
  },
  stats = {x = 3, y = 3},
  on_block = function(game, self, attacker)
    game.add_modifier(self.instance_id, "stat_boost", -2, 0, "end_of_turn")
  end,
}
