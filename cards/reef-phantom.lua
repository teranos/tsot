-- Blue 3/4 flyer. Designed around the existing attack-taps-the-creature
-- rule (B.4): attack with it → tapped → untargetable until your next
-- untap. The natural play pattern is "swing and duck" — chip damage every
-- turn while staying immune to removal during the most exposed window
-- (your opponent's turn).
--
-- Cost: 2 hand + 2 graveyard + 2 mill. The 2 graveyard gates it past
-- early game (need 2 cards already milled). The 2 hand fuels the
-- attached-blue scaling mechanic — players who pitch blue cards as the
-- HAND payment get a stat boost per blue attached.
--
-- "+1/+1 per attached blue card" wired via on_enter_board snapshot. At
-- ETB time, the engine has already attached the HAND payments (P.6) to
-- the phantom. We count attached cards whose colors include blue and
-- apply stat_boost modifiers per match. This is a SNAPSHOT — the buff
-- doesn't recompute if attached set changes later (same Phase 1.5 gap
-- as hydra's ETB stat snapshot persisting through falter strips).
--
-- Engine support:
--   - flying: enforced via has_keyword("flying") in declare_blocker.
--   - "when tapped, can't be targeted" — still NOT wired. Needs the
--     targeting layer + STATIC Phase 3 restriction statics with a
--     state-reading predicate on tapped. Today the phantom IS targetable
--     while tapped just like any other creature.
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
    {amount = 2, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "flying.",
    "this creature gets +1/+1 for every blue card attached to it.",
    "when this creature is tapped, it cannot be targeted by spells or abilities your opponents control.",
  },
  stats = {x = 3, y = 4},
  static = {
    affects = {
      scope = "source_only",
    },
    modifier = {x = "attached:blue", y = "attached:blue"},
  },
}
