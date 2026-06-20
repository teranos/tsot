-- Durian Elemental — green/cyan 3/4 reach elemental. Two abilities, both
-- non-executable in the current engine:
--
--   (1) on_turn_begin (BOARD): tap target creature, move one of its
--       attached cards to another creature. Slice A3 (TurnError::ChoicePending
--       propagation) shipped, so the handler's choose_card no longer
--       errors silently. Downstream gap per LIMITATIONS.md ## lua: the
--       StepEngine still uses RandomOracle for phase-advance triggers,
--       so the AI side gets a random answer and the human side does NOT
--       get a HumanPrompt — swapping the phase-advance oracle to
--       HumanReplayOracle is the remaining slice for human-driven play.
--       The "move one attached card between hosts" operation also needs
--       a `game.move_attached(iid, new_host)` helper, which Shift uses
--       for its X-mill effect but isn't currently exposed on the game
--       table; that's a small lua_api.rs extension.
--
--   (2) GY-zone activated (1 hand, exile this from graveyard): same
--       effect. Needs #5 (activations from non-BOARD zones; today
--       `activate.rs:79` returns NotOnBoard) and #4 (SELF in activated
--       cost: "exile this card from your graveyard" maps to the
--       SelfExile component, currently rejected at validation in
--       `activate.rs:120`).
return {
  id = "durian-elemental",
  name = "Durian Elemental",
  type = "creature",
  colors = {"green", "cyan"},
  subtypes = {"elemental"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 4, source = "graveyard"},
  },
  stats = {x = 3, y = 4},
  abilities = {
    "reach.",
    "at the beginning of your turn, tap target creature and move one of its attached cards to another creature.",
    "while this card is in your graveyard, 1H + exile this card from your graveyard: tap target creature and move one of its attached cards to another creature.",
  },
}
