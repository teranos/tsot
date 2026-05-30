-- Blue 1/1 human scientist. 1 hand + 1 graveyard. Designed as a draw-
-- replacement engine: while you control 3+ humans, any time you would
-- draw a card you draw 2 instead.
--
-- NOT WIRED today — depends on STATIC Phase 4 (replacement effects)
-- which doesn't exist yet. The replacement system needs:
--   1. An event-interception layer where the engine asks each on-board
--      static "do you want to transform this event?" before resolving.
--   2. A `static.replace` field carrying {event, handler} or an enum of
--      common replacements (draw → draw_n, would_die → exile, etc.).
--   3. APNAP ordering rules + a self-recursion guard.
--
-- Until Phase 4 lands, this card sits in pools as a 1/1 chump with a
-- 1 hand + 1 graveyard cost and printed text only. The 3-human-condition
-- itself is already expressible via Phase 2's StaticCondition system —
-- just need a new variant like OwnerBoardCountBySubtype("human", 3).
-- The replacement effect is the blocking piece.
--
-- Symbol not yet specified.
return {
  id = "blue-scientist",
  name = "Blue Scientist",
  colors = {"blue"},
  type = "creature",
  subtypes = {"human", "scientist"},
  cost = {{amount = 1, source = "hand"}, {amount = 1, source = "graveyard"}},
  abilities = {
    "if you would draw a card, draw two cards instead. only active while you control 3 or more humans.",
  },
  stats = {x = 1, y = 1},
}
