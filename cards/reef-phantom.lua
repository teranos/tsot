-- Blue 3/4 flyer. Designed around the existing attack-taps-the-creature
-- rule (B.4): attack with it → tapped → untargetable until your next
-- untap. The natural play pattern is "swing and duck" — chip damage every
-- turn while staying immune to removal during the most exposed window
-- (your opponent's turn). No explicit tap-cost ability needed; combat is
-- the activation.
--
-- Costs: 4 graveyard gates it past ~turn 4 (need 4 cards already milled),
-- 2 hand keeps it from being free-rolled. Late-game finisher, not early
-- pressure.
--
-- Engine support:
--   - flying: enforced via has_keyword("flying") in declare_blocker (only
--     flyers can block flyers per B.11).
--   - 3/4 stats: read by effective_stats; picks up any anthems and
--     contributes to the threat-aware response policy.
--   - 2 hand + 4 graveyard cost: routed by play_card today.
--   - "when tapped, can't be targeted" — NOT wired. Needs two prerequisites:
--       1. A targeting system (today game.choose_card pools are provided
--          by handlers; there's no engine-level "valid targets for this
--          effect" check that statics could intercept).
--       2. STATIC.md Phase 3 restriction statics — a conditional static
--          predicated on tapped state. Same blocker as flesh-eating-plant.
--     Until both land, the line is design intent only; today the phantom
--     IS targetable while tapped just like any other creature.
--
-- Symbol not yet specified.
return {
  id = "reef-phantom",
  name = "Reef Phantom",
  colors = {"blue"},
  type = "creature",
  subtypes = {"phantom"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 4, source = "graveyard"},
  },
  abilities = {
    "flying.",
    "when this creature is tapped, it cannot be targeted by spells or abilities your opponents control.",
  },
  stats = {x = 3, y = 4},
}
