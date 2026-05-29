-- Black/red combat trick. Cast in response to an attack declaration
-- (R.1.b window) to nuke a big incoming attacker, or in main phase as
-- a pure cantrip (no attacker → no damage, but still draws).
--
-- Designed to exercise R.1.b: today the response policy only fires on
-- R.1.a (chain-top is an opposing cast). With Sabotage in the corpus
-- and the R.1.b policy extension, the AI will cast it during combat to
-- shrink lethal threats.
--
-- 4 damage kills most creatures in the corpus (max printed Y is 5).
-- Cantrip recoups the hand cost — net resource impact = -1 (the 4 dmg).
-- Symbol not yet specified.
return {
  id = "sabotage",
  name = "Sabotage",
  colors = {"black", "red"},
  type = "instant",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "deal 4 damage to attacking creature.",
    "draw a card.",
  },
  on_play = function(game, self)
    local atks = game.attackers()
    if #atks > 0 then
      local target = game.choose_card({pool = atks, prompt = "deal 4 damage to"})
      if target then
        game.damage(target, 4)
      end
    end
    game.draw(self.owner, 1)
  end,
}
