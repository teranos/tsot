-- Black sorcery: kill target opposing creature, draw a card if it was a
-- human. First wired removal in the corpus — gives the AI a real way to
-- shrink opposing boards outside combat.
--
-- Cost: free for now (matches the existing data; balance pass is a
-- separate decision). Pre-this-handler, the card sat in the corpus as
-- text-only; the engine now does the kill + the human-rider draw.
return {
  id = "silent-murder",
  name = "Silent murder",
  colors = {"black"},
  type = "spell",
  symbol = "⊨",
  cost = {{amount = 2, source = "graveyard"}},
  abilities = {
    "kill target non-black creature. if it was a human, draw a card.",
  },
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    local board = game.zones(opp).board
    local pool = {}
    for _, iid in ipairs(board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        local is_black = false
        if c.colors then
          for _, col in ipairs(c.colors) do
            if col == "black" then
              is_black = true
              break
            end
          end
        end
        if not is_black then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "kill target creature"})
    if not target then return end
    local card = game.card(target)
    local was_human = false
    if card and card.subtypes then
      for _, st in ipairs(card.subtypes) do
        if st == "human" then
          was_human = true
          break
        end
      end
    end
    game.move(target, "graveyard")
    if was_human then
      game.draw(self.owner, 1)
    end
  end,
}
