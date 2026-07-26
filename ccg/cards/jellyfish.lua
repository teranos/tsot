-- Symbol not yet specified.
--
-- Handler: pool is opponent's board (encodes the "you probably want to bounce
-- the opponent's creature, not your own" strategy via the filter, not in the
-- oracle). The oracle picks randomly from whatever pool we pass.
return {
	id = "jellyfish",
	name = "Jellyfish",
	colors = { "blue" },
	type = "creature",
	subtypes = { "fish" },
	cost = {
		{ amount = 1, source = "hand" },
		{ amount = 2, source = "mill" },
		{ amount = 2, source = "graveyard" },
	},
	abilities = {
		"When this creature enters the board, return target creature to its owners hand.",
	},
	stats = { x = 0, y = 1 },
	on_zone_change = function(game, self, moving, from, to)
		if moving.instance_id ~= self.instance_id then return end
		if to ~= "board" then return end
		if game.host_of(self.instance_id) then return end
		local opp = game.opponent(self.owner)
		local pool = {}
		for _, iid in ipairs(game.zones(opp).board) do
			table.insert(pool, iid)
		end
		game.set_intent("remove_threat")
		local target = game.choose_card(pool, { optional = true, prompt = "bounce a creature" })
		if target then
			game.move(target, "hand")
		end
	end,
}
