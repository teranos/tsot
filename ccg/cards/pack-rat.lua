-- Black+green rat. "Can't block cats." When pack-rat dies, every rat-
-- subtype attached card on pack-rat OR on any other creature the
-- controller controls returns to the controller's hand — a tribal
-- recursion engine that turns pitched rats into recurring threats.
-- Mill cost reflects the rat-flavor "eats through your stuff" — a small
-- tax on top of the hand cost, eating the top of your deck.
--
-- The return scan reads card subtypes via game.card (face-down attached
-- cards still expose subtype data to handlers per the engine view).
-- Pack-rat's own attached is walked first; "another creature you
-- control" iterates game.zones(self.owner).board with a self-iid
-- exclusion in case the death sequence still has pack-rat on board at
-- handler-fire time. A `seen` set guards against any double-counting.
return {
	id = "pack-rat",
	name = "Pack Rat",
	type = "creature",
	colors = { "black", "green" },
	subtypes = { "rat" },
	cannot_block_subtypes = { "cat" },
	cost = {
		{ amount = 1, source = "hand" },
		{ amount = 1, source = "attach" },
		{ amount = 1, source = "graveyard" },
	},
	stats = { x = 3, y = 3 },
	abilities = {
		"can't block cats.",
		"when this creature dies, return all rat cards attached to this creature or to another creature you control to your hand.",
	},
	on_die = function(game, self)
		local seen = {}
		local to_return = {}

		local function consider(att_iids)
			for _, aid in ipairs(att_iids) do
				if not seen[aid] then
					seen[aid] = true
					local a = game.card(aid)
					if a and a.subtypes then
						for _, s in ipairs(a.subtypes) do
							if s == "rat" then
								table.insert(to_return, aid)
								break
							end
						end
					end
				end
			end
		end

		-- Pack-rat's own attached (P.8 would otherwise exile these once the
		-- death sequence finishes routing pack-rat to graveyard).
		consider(self.attached)

		-- "Another creature you control": every on-board card on owner's
		-- side of type == "creature", excluding pack-rat itself.
		for _, iid in ipairs(game.zones(self.owner).board) do
			if iid ~= self.instance_id then
				local host = game.card(iid)
				if host and host.type == "creature" and host.attached then
					consider(host.attached)
				end
			end
		end

		for _, aid in ipairs(to_return) do
			game.move(aid, "hand")
		end
	end,
}
