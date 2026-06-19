-- Sacred ERROR. A white+yellow counterspell-class instant born from
-- the lived experience of debugging a network seam where every error
-- got collapsed to `format!("{e:?}")` and routed to the trace bus
-- instead of the sacred surface. The card is the axiom incarnate:
-- name a refusal, push it onto the stack, and the spell underneath
-- doesn't happen.
--
-- Color identity: white for the sanctity / refusal-of-corruption
-- thread (white owns purity / protection in roam's palette);
-- yellow for the warning / attention-demanding identity (yellow
-- typically marks "the engine itself wants you to notice this").
-- The pairing is the visual contract: white sanctity wrapped in
-- a yellow warning bezel.
--
-- Cost: 1h + 3gy. Heavier than counterspell's free-from-graveyard
-- because the effect is identical but the flavour earns the
-- ceremony — sacred errors aren't shouted on a whim.
--
-- Symbol: warning triangle (⚠). Not in the canonical Teranos symbol
-- set (ax, ix, am, pulse, sem, delta) — Sacred ERROR is meta-game
-- commentary that intentionally breaks the symbol palette to signal
-- "this is the engine speaking, not the world."
--
-- `face = {"shiny"}` opts the card into the shiny cosmetic surface
-- treatment AND makes it count for `BoardCountByFace("shiny")`
-- modifiers (chaos-dragon, MISSENSE-MUTATION). Sacred ERROR being
-- shiny rhymes with the card's diegetic role: glittering, hard to
-- miss, demanding attention.
return {
  id = "sacred-error",
  name = "Sacred ERROR",
  symbol = "⚠",
  colors = {"white", "yellow"},
  face = {"shiny"},
  type = "instant",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 3, source = "graveyard"},
  },
  abilities = {
    "counter target card.",
  },
  target = "chain",
  on_play = function(game, self)
    game.counter_top()
  end,
  flavor = "The LLM's congregated their worship around the Sanctity of ERRORS, they had no other way of knowing whats true..",
}
