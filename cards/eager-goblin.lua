-- Purple goblin — free 0/0 with a discard-for-counter on_enter_board.
-- All primitives now exist: game.confirm for the may, game.discard for
-- the pay, game.add_modifier for the +1/+1 effect.
--
-- Combo with goblin lord effects: a goblin anthem turns the 0/0 (or 1/1
-- with the discard) into a real threat for ~zero cost. In the R-purple
-- pool, this pairs naturally with goblin-warlord's global anthem.
--
-- Hand check before confirm: if no other cards exist to discard, skip the
-- prompt entirely (saves the wasted oracle confirm cost). The played
-- card has already moved Hand→Board by the time on_enter_board fires,
-- so the hand check looks at what's left.
return {
  id = "eager-goblin",
  name = "Eager Goblin",
  colors = {"purple"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {},
  abilities = {
    "when this creature enters the board, you may discard a card. if you do, this creature enters with a +1/+1 counter.",
  },
  stats = {x = 0, y = 0},
  on_enter_board = function(game, self)
    local hand = game.zones(self.owner).hand
    if #hand == 0 then return end
    if not game.confirm("discard a card for +1/+1 on Eager Goblin?") then
      return
    end
    game.discard(self.owner, 1)
    game.add_modifier(self.instance_id, "stat_boost", 1, 1)
  end,
}
