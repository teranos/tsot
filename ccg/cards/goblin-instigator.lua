return {
  id = "goblin-instigator",
  name = "Goblin Instigator",
  colors = {"purple"},
  type = "creature",
  subtypes = {"goblin"},
  symbols = { C = "⋈", TR = "≡" },
  holes = {"TL"},
  cost = {
    {amount = 4, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "you may only cast this card if you already control a goblin.",
    "when this creature enters the board, destroy target non-purple non-goblin creature.",
  },
  stats = {x = 1, y = 1},
  on_enter_board = function(game, self)
    -- TODO(cast-validate): the "control a goblin" precondition isn't
    -- enforced at cast time yet — the cost gets paid even when no
    -- goblin is in play. Pending a Card.cast_validate hook. For now,
    -- the ETB destruction is the only effect that gates on it; if no
    -- goblin is in play the card resolves to a vanilla 1/1.
    local controls_goblin = false
    for _, iid in ipairs(game.zones(self.owner).board) do
      if iid ~= self.instance_id then
        local c = game.card(iid)
        if c then
          for _, st in ipairs(c.subtypes) do
            if st == "goblin" then controls_goblin = true; break end
          end
        end
      end
      if controls_goblin then break end
    end
    if not controls_goblin then return end

    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        local c = game.card(iid)
        if c and c.type == "creature" then
          local is_purple = false
          for _, col in ipairs(c.colors) do
            if col == "purple" then is_purple = true; break end
          end
          local is_goblin = false
          for _, st in ipairs(c.subtypes) do
            if st == "goblin" then is_goblin = true; break end
          end
          if not is_purple and not is_goblin then
            table.insert(pool, iid)
          end
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, { optional = false, prompt = "destroy a non-purple non-goblin creature" })
    if target then
      game.move(target, "graveyard")
    end
  end,
}
