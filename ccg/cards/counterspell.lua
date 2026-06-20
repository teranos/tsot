-- Blue+purple free instant. First card that uses the STACK Phase 1
-- wiring end-to-end: cast goes on the chain, response window opens,
-- counterspell goes on top via respond_with, and on resolution
-- game.counter_top() removes the chain item underneath without resolving
-- its effect.
--
-- Phase 1 implements "counter target spell" as "counter the spell
-- directly underneath me." Explicit targeting (game.counter(target)) is
-- future work; with chain depth typically 1, the two are equivalent
-- today.
--
-- Color: blue + purple. Blue for the classic control / interrupt
-- identity; purple for the chaos / disruption-of-normal-flow identity
-- (purple already owns slow-recall's exile-drip, wake-dead's haste
-- chaos, phantom-goblin's self-recur — counterspell's "no, that didn't
-- happen" sits comfortably in the unstable-magic cluster). The dual
-- color also opens deckbuilding: a B/Pu deck can pitch counterspell
-- into either zebra (B/W match — no overlap) or jewel hosts in both
-- colors.
--
-- Free cost is intentional. Pulse-glyph (꩜) fits the "interrupt"
-- semantic and now anchors the cross-color interrupt-symbol cluster
-- (counterspell, anaconda, turn-back-time, slime-beast, pink-jewel,
-- this-for-that).
return {
  id = "counterspell",
  name = "Counterspell",
  symbol = "꩜",
  colors = {"blue", "purple"},
  type = "instant",
  cost = {{amount = 1, source = "graveyard"}},
  abilities = {
    "counter target card.",
  },
  target = "chain",
  on_play = function(game, self)
    game.counter_top()
  end,
}
