-- Blue free instant. First card that uses the STACK Phase 1 wiring
-- end-to-end: cast goes on the chain, response window opens, counterspell
-- goes on top via respond_with, and on resolution game.counter_top()
-- removes the chain item underneath without resolving its effect.
--
-- Phase 1 implements "counter target spell" as "counter the spell directly
-- underneath me." Explicit targeting (game.counter(target)) is future work;
-- with chain depth typically 1, the two are equivalent today.
--
-- Free cost is intentional. Pulse-glyph (꩜) fits the "interrupt" semantic.
return {
  id = "counterspell",
  name = "Counterspell",
  symbol = "꩜",
  colors = {"blue"},
  type = "instant",
  cost = {},
  abilities = {
    "counter target spell.",
  },
  on_play = function(game, self)
    game.counter_top()
  end,
}
