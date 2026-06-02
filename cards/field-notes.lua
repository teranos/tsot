-- Blue draw instant. First card to exercise game.choose_player: caster
-- picks "target player" — almost always themselves, but the opponent is a
-- legal target (e.g., to deck them out, or to feed their handler-fires for
-- timing). Scales with combat: 3 if a creature attacked this turn, else 2.
return {
	id = "field-notes",
	name = "Field Notes",
	symbol = "⨳",
	colors = { "blue" },
	type = "instant",
	cost = {
		{ amount = 1, source = "hand" },
		{ amount = 1, source = "attached" },
	},
	abilities = {
		"target player draws three cards if a creature attacked this turn; otherwise they draw two.",
	},
	on_play = function(game, self)
		local target = game.choose_player({ prompt = "target player draws" }) or self.owner
		local n = game.creature_attacked_this_turn() and 3 or 2
		game.draw(target, n)
	end,
}
