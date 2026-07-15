-- Window Cleaner — an azure human who fields transparent sleeves.
--
-- Slice 9.3. Two triggers built on the cardless-sleeve primitives:
--   ETB (9.1): search the deck for 2 cardless sleeves and attach them.
--     Window Cleaner only ever brings CLEAR (transparent) sleeves.
--   On becoming tapped (9.2): *may* move an attached cardless sleeve to
--     the graveyard and draw. The card has no inherent tap ability — it
--     taps by attacking or by another effect, and the trigger fires on
--     the tap itself.
--
-- The loop: the two attached cardless sleeves are attach-cost fuel for
-- the next Window Cleaner (cost = 2 attach), so a chain of them keeps
-- refilling its own attach payment while the tap-trigger cantrips.
--
-- Holes T, TR, UR, R, C — the upper-right pane the cleaner keeps
-- see-through; Clear View can substitute cost components through them.
return {
  id = "window-cleaner",
  name = "Window Cleaner",
  symbol = "⨳",
  type = "creature",
  colors = {"azure"},
  subtypes = {"human"},
  holes = {"T", "TR", "UR", "R", "C"},
  cost = {
    {amount = 2, source = "attach"},
  },
  stats = {x = 2, y = 3},
  abilities = {
    "reach.",
    "when this creature enters the board, search your deck for 2 cardless sleeves and attach them to this card.",
    "when this creature becomes tapped, you may move an attached cardless sleeve to your graveyard and draw a card.",
  },
  on_enter_board = function(game, self)
    -- Z.8 search: pull up to 2 cardless sleeves out of the deck and
    -- attach them face-down. No-op if the deck holds fewer than 2.
    game.attach_cardless_from_deck(self.instance_id, self.owner, 2)
  end,
  on_tapped = function(game, self)
    -- Spend the first attached cardless sleeve, if any, for a cantrip.
    for _, aid in ipairs(self.attached) do
      if game.is_cardless(aid) then
        if game.confirm("Window Cleaner tapped — move an attached cardless sleeve to the graveyard and draw?") then
          game.move(aid, "graveyard")
          game.draw(self.owner, 1)
        end
        return
      end
    end
  end,
  flavor = "Nothing to see through here — that's the point.",
}
