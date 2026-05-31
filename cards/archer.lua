-- Green archer with reach: 2/2, ETB shoots down a flying creature.
-- Reach is a new engine keyword (lets ground creatures block flyers,
-- exception to B.11). on_play walks opponent's board for a flying
-- creature and kills it. Pattern follows silent-murder for target
-- selection + game.move(target, "graveyard").
return {
  id = "archer",
  name = "Archer",
  type = "creature",
  colors = {"green"},
  subtypes = {"archer"},
  cost = {{amount = 1, source = "hand"}},
  stats = {x = 2, y = 2},
  abilities = {
    "reach.",
    "when this creature enters the board, kill a target flying creature.",
  },
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    local pool = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" and c.abilities then
        for _, ab in ipairs(c.abilities) do
          if ab == "flying." or ab == "flying" then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "shoot down which flyer?"})
    if not target then return end
    game.move(target, "graveyard")
  end,
}
