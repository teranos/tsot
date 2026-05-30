-- Blue 2/1 human wizard. Cheap (1 hand), self-replacing (cantrip on death),
-- with conditional evasion: flying only when your graveyard has filled up
-- with non-creature cards (mid-late game). The "cannot block" restriction
-- pushes it as a one-way attacker — chip damage in the air, replace itself
-- when traded off.
--
-- Synergy notes:
--   - Battle-captain's anthem (other humans you control get +1/+1) buffs
--     this to 3/2 — flying 3/2 with cantrip-on-death is real value for 1 hand.
--   - U-variant decks (heavy on instants/sorceries) fill the graveyard
--     fast, so the flying condition triggers earlier for them.
--
-- Engine support:
--   - 2/1 stats, blue color, human/wizard subtypes: ✅ read by effective_stats
--     and the anthem system.
--   - 1 hand cost: ✅ routed by play_card.
--   - on_die cantrip: ✅ wired (game.draw is a primitive).
--   - flying (conditional on graveyard composition): ❌ pending. Needs
--     STATIC Phase 2 (keyword grants) + a state-reading predicate (the
--     declarative Phase 1 affects struct doesn't express "owner's graveyard
--     has > N non-creature cards"). When wired, the static would be a
--     self-targeting keyword grant whose predicate evaluates on every
--     has_keyword call (lazy eval already accommodates this).
--   - cannot block: ❌ pending. Same blocker as flesh-eating-plant —
--     STATIC.md Phase 3 restriction statics. New keyword "cannot-block"
--     would be enforced in declare_blocker.
--
-- Symbol not yet specified.
return {
  id = "wandering-wizard",
  name = "Wandering Wizard",
  colors = {"blue"},
  type = "creature",
  subtypes = {"human", "wizard"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "this creature has flying when more than 3 non-creature cards are in your graveyard.",
    "cannot-block.",
    "this creature cannot block.",
    "when this creature dies, draw a card.",
  },
  stats = {x = 2, y = 1},
  on_die = function(game, self)
    game.draw(self.owner, 1)
  end,
}
