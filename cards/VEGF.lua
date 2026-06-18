-- TODO: you,know, i want the mutation type card to actually behave differently from how we do attached cards. i think mutations are stickier, so, this means that in my view the mutation should stay with the cerature for the duration of the game, so the card always moves together with the creatuire, and also, i dont think that if a creature ever gets a mutation, that it should ever lose it, if you catch my draft. so a mutated creaturecan only accumulate mutations..
--
return {
	id = "VEGF",
	name = "VEGF",
	type = "mutation",
	colors = { "blue", "black" },
	cost = {},
	abilities = {
		"the host creature gets: whenever this creature attacks, mill 3 cards.",
	},
	on_attack = function(game, self)
		-- TODO: Why do we specify this like this? like, why do we need to set "graveyard" ?? in my mental image mill always goes to gy.
		game.mill(self.owner, 3, "graveyard")
	end,
}
