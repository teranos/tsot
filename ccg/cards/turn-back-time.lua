-- Pink sorcery: roll back time. Pink's flagship — the color of "no,
-- that didn't happen." X comes from the X-sacrifice cost (sacrifice
-- creatures from your board; X turns get reverted = current turn + the
-- X-1 prior turns). Self-exile cost ensures each copy is one-shot.
--
-- Path A (chosen 2026-05-31): the rewind reverts EVERYTHING in the
-- target window, including the cost-payment of Turn Back Time itself.
-- This means in the new timeline:
--   - the X sacrifices come back to your board (those deaths didn't
--     happen)
--   - the 1 hand payment returns to your hand (that play didn't
--     happen)
--   - all plays / combat / mill / damage / draws of the rewound turns
--     are undone
--
-- The ONE anchor: Turn Back Time itself stays exiled. The self-exile
-- cost would be reverted by the naive rewind, so the handler will
-- explicitly re-exile the card via game.move(self.instance_id, "exile")
-- AFTER calling game.rewind(X). One copy = one rewind, forever.
-- Time-travel paradox neatly side-stepped: the card is the only thing
-- "outside" the rewound timeline.
--
-- Engine work pending:
--   - game.rewind(n) Lua primitive — walks replay_journal backwards,
--     crossing n-1 SetTurn boundaries, applying inverses entry-by-entry.
--     The Journal::rollback infrastructure (consume-and-reverse) already
--     exists; rewind_turns is the partial / truncating variant.
--   - is_x = true on a sacrifice cost component needs the sim AI's
--     sacrifice picker (sim/ai.rs) to honor X-scaled slots; today it
--     reads c.amount directly without expanding by x_value.
--   - The cost itself parses today (cost validation accepts is_x +
--     sacrifice) — the card LOADS, it just can't be cast yet. Handler
--     intentionally absent so a cast doesn't blow up trying to call a
--     non-existent game.rewind. Lands now as design-anchored data; the
--     wiring is the next branch's work.
--
-- Color: pink. Inaugurates the 7th color. The engine accepts arbitrary
-- color strings (per commit f9c650d) so no engine change needed for
-- the color itself, only for the cost-modifier statics and EA variant
-- pools that may want to learn pink later.
return {
  id = "turn-back-time",
  name = "Turn Back Time",
  symbol = "꩜",
  colors = {"pink"},
  type = "spell",
  cost = {
    {amount = 1, source = "hand"},
    {is_x = true, source = "sacrifice"},
    {amount = 1, source = "self"},
  },
  abilities = {
    "Revert the current turn. For each creature sacrificed beyond the first (X-1), also revert the previous turn. This card is exiled when cast and is not restored by the rewind.",
  },
}
