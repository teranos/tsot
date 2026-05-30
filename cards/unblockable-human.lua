-- 0/1 unblockable human. Design intent: "always correct to attack." The
-- body deals 0 damage to the deck (X = 0) so combat is risk-free —
-- nothing can block it and the attack itself can't be punished. The
-- payoff is the on_attack loot: discard one and draw one. Strictly free
-- card filtering paid per turn, with no opportunity cost (X=0 means
-- holding it back as a blocker is also worthless).
--
-- The AI's `is_attack_worth_declaring` short-circuits to TRUE on
-- unblockable, so this creature attacks every turn it's legal. Wiring
-- the loot adds real value to each attack.
--
-- Unwired (engine gap, not card gap):
--   "when this creature attacks you may exile a card from your graveyard;
--    if you do, this creature gets +2/+0 until end of turn." — needs the
--   temporary stat-modifier system (no time-bound +X/+0 today). Once that
--   lands, this becomes the threat half (mill 2/turn instead of 0).
--
-- Symbol not yet specified.
return {
  id = "unblockable-human",
  name = "Unblockable Human",
  type = "creature",
  colors = {"blue"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "unblockable.",
    "when this creature attacks you may exile a card from your graveyard; if you do, this creature gets +2/+0 until end of turn.",
    "when this creature attacks a player you may discard a card and draw a card.",
  },
  stats = {x = 0, y = 1},
  on_attack = function(game, self)
    -- Loot: always take it. It's strictly card filtering, never worse
    -- than holding the discarded card. The "may" wording in the printed
    -- text is courtesy — no realistic line passes on this trigger.
    game.discard(self.owner, 1)
    game.draw(self.owner, 1)
  end,
}
