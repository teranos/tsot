-- Blue mutation: whenever the host creature deals combat damage to a
-- player, draw a card. Free to cast (no cost).
--
-- Named for Klotho, the Greek Fate who spins the thread of life — the
-- mutation accelerates a creature's "thread" (its damage-to-draw cycle).
--
-- NOT WIRED — depends on the missing `OnDealtDamageToPlayer` event
-- (LIMITATIONS-listed; cinder-wurm + bci-megafly use on_attack as a
-- workaround). Once the event lands, the cleanest pattern is a mutation-
-- side `on_host_event` indirection — the engine fires the mutation's
-- handler whenever the relevant event fires on its host. Same shape
-- companion-bird's static uses, but for events rather than continuous
-- effects.
--
-- Free cost makes Klotho an always-cast-if-you-have-a-creature card
-- once it's wired. A creature can carry multiple mutations — each one
-- attaches via add_attached and each one's static fires independently
-- from the host's attached list. Stacking +1/+1 mutations + draw-on-
-- damage mutations on a single body is the natural endgame.
return {
  id = "klotho",
  name = "Klotho",
  type = "mutation",
  colors = {"blue"},
  cost = {},
  abilities = {
    "the host creature gets: whenever this creature deals damage to a player, draw a card.",
  },
}
