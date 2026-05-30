-- Blue mutation: whenever the host creature deals combat damage to a
-- player, draw a card.
--
-- NOT WIRED — depends on the missing `OnDealtDamageToPlayer` event
-- (LIMITATIONS-listed; cinder-wurm + bci-megafly use on_attack as a
-- workaround for similar texts). Once that event lands, the handler
-- would attach to the HOST (via OnDealtDamageToPlayer fired on the host,
-- with self = host but mutation's handler firing because the mutation
-- is in the host's attached list).
--
-- Cleaner pattern: a mutation-side "on_host_event" indirection — the
-- engine fires the mutation's handler whenever the relevant event fires
-- on its host. Same shape companion-bird's static uses, but for events
-- rather than continuous effects.
return {
  id = "accelerated-thought",
  name = "Accelerated Thought",
  type = "mutation",
  colors = {"blue"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "the host creature gets: whenever this creature deals damage to a player, draw a card.",
  },
}
