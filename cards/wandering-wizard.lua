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
-- Blue 2/1 human wizard. Cheap (1 hand), self-replacing (cantrip on death),
-- with conditional evasion: flying only when your graveyard has filled up
-- with non-creature cards (mid-late game). The "cannot block" pushes it
-- as a one-way attacker — chip damage in the air, replace itself on trade.
--
-- Synergy: battle-captain (other humans +1/+1) buffs to 3/2 flying with
-- cantrip-on-death. U-variant decks fill the graveyard with spells fast,
-- triggering the flying condition earlier.
--
-- Conditional flying wired via STATIC Phase 2: `scope = "source_only"`
-- targets the wizard itself; `condition.owner_graveyard_non_creatures
-- min=4` is "> 3" non-creature cards. `cannot-block` is intrinsic (B.18).
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
    "when this creature dies, draw a card.",
  },
  stats = {x = 2, y = 1},
  static = {
    affects = {
      scope = "source_only",
    },
    modifier = {keyword = "flying"},
    condition = {kind = "owner_graveyard_non_creatures", min = 4},
  },
  on_die = function(game, self)
    game.draw(self.owner, 1)
  end,
}
