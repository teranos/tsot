-- Pink instant: 1 graveyard. Target an attacking creature and put it
-- on the bottom of its owner's deck. Hard-removes the attack and
-- recycles the creature far down the deck — opp draws it again only
-- after most of their deck has cycled through.
return {
  id = "lullaby",
  name = "Lullaby",
  colors = {"pink"},
  type = "instant",
  cost = {{amount = 1, source = "graveyard"}},
  abilities = {
    "target attacking creature is placed on the bottom of its owner's deck.",
  },
  on_play = function(game, self)
    local pool = game.attackers()
    if #pool == 0 then return end
    game.set_intent("remove_threat")
    local target = game.choose_card(pool, {prompt = "lullaby which attacker?"})
    if not target then return end
    game.move(target, "deck")
  end,
}
