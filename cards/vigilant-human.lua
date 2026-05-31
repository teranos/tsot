-- White human 2/2 with vigilance + a T-ability that pays off attacking.
-- The synergy: vigilance keeps the creature untapped through combat, so
-- post-combat the activation is still live. Activation gates on whether
-- THIS creature actually attacked (engine tracks per-instance via
-- `attacked_this_turn`, exposed on game.card). If it didn't attack, the
-- tap pays for nothing — the AI's post-combat activation pass will only
-- fire abilities that can_activate, but won't reason about whether the
-- effect will no-op. Practical impact: minor; the smart-attacker also
-- decides whether to swing, and if it skips the swing the AI mostly
-- still activates (a wasted tap, but no card lost).
return {
  id = "vigilant-human",
  name = "Vigilant Human",
  type = "creature",
  colors = {"white"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}, {amount = 1, source = "graveyard"}},
  abilities = {
    "vigilance.",
    "T: if this creature attacked this turn, draw a card.",
  },
  stats = {x = 2, y = 2},
  activated = {
    {
      cost = "tap",
      text = "T: if this creature attacked this turn, draw a card.",
      timing = "instant",
      effect = function(game, self)
        local me = game.card(self.instance_id)
        if me and me.attacked_this_turn then
          game.draw(self.owner, 1)
        end
      end,
    },
  },
}
