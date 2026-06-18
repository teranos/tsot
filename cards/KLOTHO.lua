-- Blue mutation: whenever the host creature successfully damages a
-- player, draw a card. Free to cast (no cost).
--
-- Named for Klotho, the Greek Fate who spins the thread of life. In
-- biology, Klotho is also a real anti-aging gene/protein — the start
-- of a small "protein name" mutation cycle alongside FST.
--
-- Wired via `on_dealt_damage_to_player`. The combat resolver fires
-- this event on every card attached to an attacker that successfully
-- damaged the defender's deck (B.2). Klotho's handler receives `self`
-- = the Klotho mutation; it draws a card for `self.owner`.
return {
  id = "klotho",
  name = "Klotho",
  type = "mutation",
  colors = {"blue"},
  cost = {},
  abilities = {
    "the host creature gets: whenever this creature deals damage to a player, draw a card.",
  },
  on_dealt_damage_to_player = function(game, self)
    game.draw(self.owner, 1)
  end,
}
