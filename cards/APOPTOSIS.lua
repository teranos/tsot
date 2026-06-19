-- APOPTOSIS — yellow/purple mutation. Programmed cell death by orderly
-- dismantling: each turn one of the host's attached cards goes to
-- graveyard; when the host has no attached cards left, the host is
-- sacrificed. The mutation itself is inside the sleeve (`same_sleeve`)
-- and is NOT counted as an attached card — it does not strip itself,
-- and once the real attached cards are exhausted the host dies even
-- though the mutation is still "with" it.
--
-- Non-executable today, dependencies:
--   - Slice A3 (turn.rs:119 ChoicePending discard) — the on_turn_begin
--     trigger fires `game.choose_card` to pick which attached card to
--     strip; today that yield is discarded and the handler errors.
--   - `same_sleeve` semantic on the mutation (not yet in the engine).
--     Without it, the mutation IS in the attached list, and the "no
--     attached cards remain → sacrifice host" check would either count
--     the mutation as still-attached (host never dies) or strip the
--     mutation as the last attached card (host dies but mutation goes
--     too, violating the sleeve metaphor). With `same_sleeve = true`,
--     the engine excludes this mutation from "is anything attached"
--     checks and from "pick an attached card to move/strip" pools, and
--     keeps the mutation on the host through host death (mutation moves
--     to graveyard with the host, doesn't get exiled per P.8).
return {
  id = "APOPTOSIS",
  name = "APOPTOSIS",
  type = "mutation",
  colors = {"yellow", "purple"},
  same_sleeve = true,
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "the host creature gets: at the beginning of your turn, move one of this creature's attached cards to your graveyard. if no cards are attached to this creature anymore, sacrifice it.",
  },
  flavor = "The cell tidies itself out of existence, one organelle at a time.",
}
