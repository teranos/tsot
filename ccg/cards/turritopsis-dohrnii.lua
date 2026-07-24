return {
	id = "turritopsis-dohrnii",
	name = "Turritopsis Dohrnii",
	type = "creature",
	colors = { "azure" },
	subtypes = { "jellyfish" },
	cost = { { amount = 2, source = "hand" } },
	face = { "glow" },
	abilities = {
		"glow.",
		"when Turritopsis Dohrnii is put into your graveyard from your deck, put it on the board.",
	},
	stats = { x = 1, y = 1 },
	flavor = "The jellyfish that lives forever.",

	on_zone_change = function(game, self, moving, from, to)
		if moving.instance_id ~= self.instance_id then
			return
		end
		if from == "deck" and to == "graveyard" then
			game.move(self.instance_id, "board")
		end
	end,
}
