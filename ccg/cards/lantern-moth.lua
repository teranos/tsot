return {
	id = "lantern-moth",
	name = "Lantern Moth",
	type = "creature",
	colors = { "white" },
	subtypes = { "insect" },
	cost = { { amount = 2, source = "hand" } },
	-- BL outer so its own symbol dodges common C-holes, 5 holes for the wings
	symbols = { BL = "꩜" },
	holes = { "C", "U", "UL", "UR", "T" },
	face = { "glow" },
	abilities = {
		"flying.",
		"glow.",
		"when Lantern Moth enters the board, look at the top 2 cards of opponent's deck and put them back in any order.",
		"whenever a glow card leaves the board, draw a card.",
	},
	stats = { x = 0.5, y = 3 },

	on_zone_change = function(game, self, moving, from, to)
		-- Self ETB: look at opponent's top 2 and reorder.
		if moving.instance_id == self.instance_id and to == "board" and not game.host_of(self.instance_id) then
			local opp = game.opponent(self.owner)
			local deck = game.zones(opp).deck
			if not deck or #deck < 2 then
				return
			end
			local a = deck[1]
			local b = deck[2]
			local ca = game.card(a)
			local cb = game.card(b)
			if ca and cb then
				game.print("Top 2: " .. ca.id .. ", " .. cb.id)
			end
			local pick = game.choose_card({ a, b }, { prompt = "choose card to put on top" })
			if pick == a then
				game.move_to_deck_top(b)
				game.move_to_deck_top(a)
			else
				game.move_to_deck_top(a)
				game.move_to_deck_top(b)
			end
			return
		end
		-- Watcher: any glow card leaves the board → draw.
		if from ~= "board" then
			return
		end
		local c = game.card(moving.instance_id)
		if not c or not c.face then
			return
		end
		for _, fa in ipairs(c.face) do
			if fa == "glow" then
				game.draw(self.controller, 1)
				return
			end
		end
	end,
}
