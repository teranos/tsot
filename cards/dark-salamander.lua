-- Black salamander. Cast: X hand cards attach to the salamander
-- (hydra pattern). Effective stats are X/X via the source-only static
-- reading the attached count — so the salamander you cast for X = 3
-- arrives as a 3/3.
--
-- Activated ability (DEFERRED):
--   "Y hand: mill your opponent by 2Y - X"
-- where Y is the activation's variable cost (cards discarded from your
-- hand) and X is the salamander's effective X stat at activation time.
-- Mill scales aggressively with Y: even Y = 1 mills `2 - X`, and the
-- payoff per Y is 2 cards (not 1). Bigger body = harder to break even
-- on small activations, but big-Y activations stay efficient.
--
-- Two engine pieces missing before the activation fires:
--   1. **X-cost activations (Phase 1.75)** — the activation cost shape
--      `{{is_x = true, source = "hand"}}` parses but `activate_ability`
--      doesn't pay variable amounts yet; the activation pass would call
--      with X = 0 (the default amount on an is_x component).
--   2. **Handler-side access to the activation's Y value** — handlers
--      need something like `game.x_value()` to read the X paid for the
--      activation. The cast-time analogue lives in `PlayChoices.x_value`
--      but isn't exposed to Lua. New API plus an `ActivateChoices` shape.
--
-- Until both land, the card loads with the cost shape captured and the
-- ability text describing intent, but the `activated` table is omitted
-- so the engine doesn't try to fire a partial handler.
--
-- Sim AI caveat: also depends on the X-rejection fix in
-- `pick_random_playable_in_hand` for non-creature casts; creature casts
-- with is_x bypass that gate today (same as hydra), so the salamander
-- IS playable in the sim as a base 0/0 that grows via attached count.
return {
  id = "dark-salamander",
  name = "Dark Salamander",
  symbol = "IX",
  colors = {"black"},
  type = "creature",
  subtypes = {"salamander"},
  cost = {{is_x = true, source = "hand"}},
  abilities = {
    "Y hand: mill your opponent by Y + Y - X.",
  },
  stats = {x = 0, y = 0},
  static = {
    affects = {
      scope = "source_only",
    },
    modifier = {x = "attached", y = "attached"},
  },
}
