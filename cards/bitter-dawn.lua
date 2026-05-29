-- White board wipe with delayed self-cost. Deferred across multiple gaps:
--
--   1. Effect 1 ("kill all attacking creatures") is a response-instant —
--      it's cast during opponent's combat. Without the stack/response window
--      (STACK theme), only the active player plays cards, so the effect
--      would self-destruct the controller's own attackers.
--   2. Effect 2 ("creatures sacrificed at end of your next turn") needs a
--      delayed-trigger registry, `on_turn_end` (LUA Phase 3), and the
--      Sacrifice cost source / engine action.
--
-- Until those land, this card sits in the corpus as data + abilities text.
return {
  id = "bitter-dawn",
  name = "Bitter Dawn",
  colors = {"white"},
  type = "instant",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "kill all attacking creatures.",
    "during your next turn, creatures you control must be sacrificed at the end of the turn.",
  },
}
