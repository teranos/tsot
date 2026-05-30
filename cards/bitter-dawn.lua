-- White board wipe — instant-speed kill of all declared attackers. Cast
-- during opponent's R.1.b combat-trick window to wipe their swing, or
-- in your own main phase as a pre-emptive (no attackers → no-op + cost
-- spent).
--
-- Effect 1 ("kill all attacking creatures") — ✅ wired. on_play iterates
-- game.attackers() and moves each to graveyard. Self-targeting attackers
-- are NOT excluded — RAW the card kills every attacker including yours
-- if you happened to be the active player. With R.1.b windows opening
-- during the OPPONENT's combat (after they declared attackers), the
-- common path is "I cast this in response to their swing, their
-- attackers die." The response policy already gates on opposing combat
-- threats (would_die_soon + combat_threat), so the AI casts this when
-- opponent's incoming damage matters.
--
-- Effect 2 ("creatures sacrificed at end of your next turn") — ❌ still
-- deferred. Needs delayed-trigger registry + on_turn_end event + the
-- Sacrifice cost source. None of those exist yet.
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
  on_play = function(game, self)
    for _, iid in ipairs(game.attackers()) do
      game.move(iid, "graveyard")
    end
  end,
}
