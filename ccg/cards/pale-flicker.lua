return {
	id = "pale-flicker",
	name = "Pale Flicker",
	type = "creature",
	colors = { "white" },
	subtypes = { "spirit" },
	-- 1 sacrifice instead of 1 hand - P.16
	cost = { { amount = 1, source = "sacrifice", criterion = "creature" } },
	symbols = { UL = "⟁" },
	holes = { "C", "U" },
	face = { "glow" },
	abilities = {
		"glow.",
		"when Pale Flicker enters the board, exile target glow creature you control. Return it to the board at the beginning of your next main phase.",
	},
	stats = { x = 3, y = 4 },

	on_enter_board = function(game, self)
		local pool = {}
		for _, iid in ipairs(game.zones(self.controller).board) do
			if iid ~= self.instance_id then
				local c = game.card(iid)
				if c and c.type == "creature" and c.face then
					for _, fa in ipairs(c.face) do
						if fa == "glow" then
							table.insert(pool, iid)
							break
						end
					end
				end
			end
		end
		if #pool == 0 then
			return
		end
		local target = game.choose_card(pool, { prompt = "flicker which glow?" })
		if not target then
			return
		end
		game.move(target, "exile")
		game.schedule_return_at_next_main(target)
	end,
}
