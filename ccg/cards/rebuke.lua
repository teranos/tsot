return {
	id = "rebuke",
	name = "Rebuke",
	type = "spell",
	timing = "sorcery",
	colors = { "red" },
	cost = { { amount = 1, source = "self" } },
	abilities = {
		"for every card put into a graveyard this turn, target player mills two cards. self-exile per P.5.",
	},
	on_play = function(game, self)
		local target = game.choose_player({
			prompt = "who mills?",
		})
		if not target then
			return
		end
		local n = game.graveyard_added_this_turn()
		if n <= 0 then
			return
		end
		game.mill(target, n * 2, "graveyard")
	end,
	flavor = "Every burial owes a debt.",
}
