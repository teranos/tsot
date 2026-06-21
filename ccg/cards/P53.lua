-- Guardian of the genome. Reads cellular stress and selects a response:
-- APOPTOSIS (death), SENESCENCE (arrest), or HYPOXIA (oxygen-starvation).
return {
  id = "P53",
  name = "P53",
  type = "spell",
  colors = {"purple"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "search your deck for APOPTOSIS, SENESCENCE, or HYPOXIA and put it in your hand.",
  },
  flavor = "Reads the damage, picks the response.",
  on_play = function(game, self)
    local targets = {"APOPTOSIS", "SENESCENCE", "HYPOXIA"}
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c then
        for _, target_id in ipairs(targets) do
          if c.id == target_id then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool == 0 then return end
    local picked = game.choose_card(pool, {
      optional = true,
      prompt = "P53: select stress response",
    })
    if picked == nil then return end
    game.move(picked, "hand")
  end,
}
