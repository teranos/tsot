-- Green/purple toad with the pulse symbol. Activates by tapping to
-- "destroy" a target insect on either BOARD — but the destruction is
-- redirected: the insect leaves the BOARD and attaches to this toad
-- face-down (P.17). No on_die fires (it's a routing override, not a
-- normal death). Hydra-style stat scaling reads the attached count,
-- so consuming insects grows the toad over time.
return {
  id = "happy-toad",
  name = "Happy Toad",
  colors = {"green", "purple"},
  symbol = "꩜",
  type = "creature",
  subtypes = {"toad"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  stats = {x = 1, y = 3},
  abilities = {
    "T: destroy target insect. instead of it going to the graveyard, attach it to this creature.",
  },
  activated = {
    {
      cost = "tap",
      text = "T: destroy target insect; attach it to this creature instead of going to GRAVEYARD.",
      timing = "instant",
      validate = function(game, self)
        for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
          for _, iid in ipairs(game.zones(side).board) do
            local c = game.card(iid)
            if c and c.subtypes then
              for _, st in ipairs(c.subtypes) do
                if st == "insect" then
                  return true
                end
              end
            end
          end
        end
        return false
      end,
      effect = function(game, self)
        local pool = {}
        for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
          for _, iid in ipairs(game.zones(side).board) do
            local c = game.card(iid)
            if c and c.subtypes then
              for _, st in ipairs(c.subtypes) do
                if st == "insect" then
                  table.insert(pool, iid)
                  break
                end
              end
            end
          end
        end
        if #pool == 0 then return end
        local target = game.choose_card(pool, {prompt = "destroy and attach which insect?"})
        if target then
          game.attach(self.instance_id, target)
        end
      end,
    },
  },
}
