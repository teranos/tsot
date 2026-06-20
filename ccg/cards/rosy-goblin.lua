return {
  id = "rosy-goblin",
  name = "Rosy Goblin",
  symbol = "꩜",
  colors = {"pink"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  stats = {x = 1, y = 1},
  abilities = {
    "when this creature attacks, other goblins you control get +1/+0 until end of turn.",
  },
  on_attack = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).board) do
      if iid ~= self.instance_id then
        local c = game.card(iid)
        if c and c.subtypes then
          for _, st in ipairs(c.subtypes) do
            if st == "goblin" then
              game.add_modifier(iid, "stat_boost", 1, 0, "end_of_turn")
              break
            end
          end
        end
      end
    end
  end,
}
