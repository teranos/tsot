-- Name and symbol not yet specified.
-- Variable-X cost: X hand cards become attached. ETB applies a one-shot
-- +N/+N StatBoost where N = number of attached cards at play time.
--
-- Note: hydra's printed ability is "+1/+1 per attached card" which is
-- technically static (re-evaluates if attached changes). The ETB-modifier
-- below is a deterministic snapshot — if attached cards are later removed
-- or added, hydra's stats don't update. Phase 2 LUA `static` will close
-- this gap when it lands.
return {
  id = "hydra",
  colors = {"green"},
  type = "creature",
  subtypes = {"hydra"},
  cost = {{is_x = true, source = "hand"}},
  abilities = {
    "this creature gets +1/+1 for each attached card.",
  },
  stats = {x = 0, y = 0},
  on_enter_board = function(game, self)
    local n = #self.attached
    if n > 0 then
      game.add_modifier(self.instance_id, "stat_boost", n, n)
    end
  end,
}
