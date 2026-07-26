-- Azure + white artifact. A vitrification vessel: when it enters the
-- board it pulls one creature off the field, freezes it in stasis
-- inside the chamber, and holds it there as long as the chamber sits.
-- When the chamber leaves play, the held creature thaws at the next
-- main phase and returns to the board.
--
-- TDD slice 1: card data only — colors / type / cost / holes. The ETB
-- exile-and-attach handler, the on-leave-play scheduler, and the
-- delayed-return primitive land in subsequent slices.
return {
	id = "cryogenic-chamber",
	name = "Cryogenic Chamber",
	type = "artifact",
	colors = { "white", "azure" },
	holes = { "L", "R", "T", "TR", "B", "BL" },
	cost = {
		{ amount = 1, source = "graveyard" },
	},
	abilities = {
		"when this card enters the board, choose target creature on either board and attach it face-down to this card.",
		"when this card leaves play, return the attached card to its owner's board at the start of the next main phase (Main1 or Main2 of any player's turn, whichever comes first).",
	},
	flavor = "Vitrified. The clock waits with it.",
	on_zone_change = function(game, self, moving, from, to)
		if moving.instance_id ~= self.instance_id then return end
		if to == "board" then
			if game.host_of(self.instance_id) then return end
			-- ETB: build the pool of every creature on either board (excluding
			-- the chamber itself; belt-and-suspenders) and freeze the pick.
			local pool = {}
			for _, side in ipairs({ self.owner, game.opponent(self.owner) }) do
				for _, iid in ipairs(game.zones(side).board) do
					if iid ~= self.instance_id then
						local c = game.card(iid)
						if c and c.type == "creature" then
							table.insert(pool, iid)
						end
					end
				end
			end
			if #pool == 0 then
				return
			end
			local target = game.choose_card(pool, {
				prompt = "Freeze a creature inside Cryogenic Chamber",
				optional = false,
			})
			if target then
				-- game.attach moves the target from its BOARD slot into the
				-- chamber's `attached` list, face-down per P.17. The chamber
				-- remembers the held card through its attached list.
				game.attach(self.instance_id, target)
			end
		elseif from == "board" and to == "graveyard" then
			-- Chamber died: queue each held card for return at next main phase.
			-- After this handler runs, P.8 cascades any remaining attached
			-- cards into EXILE — that's where the queued iids live until the
			-- turn loop flushes them back to their owner's board.
			for _, iid in ipairs(self.attached) do
				game.schedule_return_at_next_main(iid)
			end
		end
	end,
}
