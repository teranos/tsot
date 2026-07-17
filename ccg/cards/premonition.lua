-- Premonition — an azure instant whose payoff arrives a turn late.
--
-- The first card to use the delayed-trigger registry (slice-11 follow-up).
-- It illustrates what that registry uniquely enables: a delayed effect
-- on a card that is NOT a board permanent. Premonition resolves and goes
-- to the graveyard immediately; on_play only SCHEDULES the draw, and the
-- draw fires at the start of your next turn — from the graveyard, via
-- game.schedule_next_turn + on_delayed_trigger. An on_turn_begin trigger
-- couldn't express this: the card isn't on the board to carry one.
--
-- Azure for the "see what's coming" identity that Window Cleaner's
-- clear-sleeve cluster already owns.
return {
  id = "premonition",
  name = "Premonition",
  symbol = "⊨",
  colors = {"azure"},
  type = "instant",
  cost = {
    {amount = 1, source = "hand"},
  },
  abilities = {
    "at the beginning of your next turn, draw three cards.",
  },
  on_play = function(game, self)
    -- Don't draw now — schedule the draw for the start of your next turn.
    game.schedule_next_turn(self.instance_id)
  end,
  on_delayed_trigger = function(game, self)
    game.draw(self.owner, 3)
  end,
  flavor = "You already knew.",
}
