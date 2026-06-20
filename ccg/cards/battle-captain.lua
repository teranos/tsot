-- Battle-captain: white human with two abilities, both wired:
--   - Static anthem: other humans you control get +1/+1 (STATIC.md Phase 1).
--   - on_attack: untap all other creatures you control that are attacking.
return {
	id = "battle-captain",
	name = "Battle Captain",
	colors = { "white" },
	type = "creature",
	subtypes = { "human" },
	cost = { { amount = 1, source = "hand" }, { amount = 1, source = "graveyard" } },
	abilities = {
		"all other humans you control get +1/+1.",
		"whenever this creature attacks, untap all other creatures you control that are attacking.",
	},
	stats = { x = 2, y = 2 },
	static = {
		affects = {
			subtypes = { "human" },
			controller = "owner",
			exclude_self = true,
		},
		modifier = { x = 1, y = 1 },
	},
	on_attack = function(game, self)
		for _, iid in ipairs(game.attackers()) do
			if iid ~= self.instance_id then
				game.untap(iid)
			end
		end
	end,
}
