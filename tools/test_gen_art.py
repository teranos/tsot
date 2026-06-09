#!/usr/bin/env python3
"""Tests for tools/gen_art.py.

Run: python3 -m unittest tools.test_gen_art
  or python3 tools/test_gen_art.py
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gen_art  # noqa: E402


class SlotRectTests(unittest.TestCase):
    def test_center(self):
        self.assertEqual(gen_art.slot_rect("C"), (128, 256, 128, 128))

    def test_top_left(self):
        self.assertEqual(gen_art.slot_rect("TL"), (0, 0, 128, 128))

    def test_bottom_right(self):
        self.assertEqual(gen_art.slot_rect("BR"), (256, 512, 128, 128))

    def test_upper(self):
        self.assertEqual(gen_art.slot_rect("U"), (128, 128, 128, 128))

    def test_unknown_raises(self):
        with self.assertRaises(ValueError):
            gen_art.slot_rect("ZZ")

    def test_grid_covers_full_canvas(self):
        # 15 slots × 128² each = 384 × 640 exact.
        names = [n for row in gen_art.SLOT_GRID for n in row]
        self.assertEqual(len(names), 15)
        self.assertEqual(len(set(names)), 15)


class HolesAestheticTests(unittest.TestCase):
    def test_pale_apparition_vertical_wraith(self):
        holes = ["TL", "UL", "L", "DL", "BL", "TR", "UR", "R", "DR", "BR"]
        out = gen_art.holes_aesthetic(holes)
        self.assertIn("vertical", out.lower())

    def test_glass_moth_glass_body(self):
        holes = ["BR", "B", "BL", "DL", "L", "UL", "TL"]
        out = gen_art.holes_aesthetic(holes)
        self.assertIn("glass", out.lower())

    def test_signal_goblin_single_center_void(self):
        out = gen_art.holes_aesthetic(["C"])
        self.assertIn("central", out.lower())
        self.assertIn("void", out.lower())

    def test_empty_returns_empty(self):
        self.assertEqual(gen_art.holes_aesthetic([]), "")


class BuildPromptTests(unittest.TestCase):
    def _tusker(self):
        return {
            "id": "tusker", "name": "Tusker", "type": "creature",
            "colors": ["orange"], "subtypes": ["elephant"],
            "abilities": [],
        }

    def test_tusker_includes_name_and_subtype(self):
        prompt, _neg = gen_art.build_prompt(self._tusker())
        self.assertIn("Tusker", prompt)
        self.assertIn("elephant", prompt)

    def test_tusker_includes_orange_palette(self):
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertIn("ember", prompt.lower())

    def test_single_color_card_repeats_color_word(self):
        # Color enforcement: the card's color word should appear multiple times
        # in the prompt so SD's token-attention biases toward that hue.
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertGreaterEqual(prompt.lower().count("orange"), 2)

    def test_colorless_card_uses_neutral_palette(self):
        card = {"id": "x", "name": "X", "type": "creature",
                "colors": [], "subtypes": ["thing"], "abilities": []}
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("achromatic", prompt.lower())

    def test_every_prompt_has_style_suffix(self):
        prompt, _ = gen_art.build_prompt(self._tusker())
        # Style anchor that survived the fauvist/psychedelic/riso removal.
        self.assertIn("ralph steadman", prompt.lower())

    def test_prompt_marks_full_bleed(self):
        # Pure "Trading Card Game" tokens bias SD into drawing a card frame
        # inside the art. We anchor on "full-bleed" instead.
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertIn("full-bleed", prompt.lower())

    def test_prompt_does_not_say_trading_card_game(self):
        # The phrase was making SD render a literal card-with-frame.
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertNotIn("trading card game", prompt.lower())

    def test_negative_excludes_inner_borders(self):
        _, neg = gen_art.build_prompt(self._tusker())
        self.assertIn("border", neg.lower())

    def test_prompt_includes_card_type(self):
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertIn("creature", prompt.lower())

    def test_prompt_includes_cost_type_graveyard(self):
        card = {"id": "x", "name": "X", "type": "creature",
                "colors": ["red"], "subtypes": ["spirit"],
                "abilities": [], "cost": [
                    {"amount": 1, "source": "hand"},
                    {"amount": 4, "source": "graveyard"},
                ]}
        prompt, _ = gen_art.build_prompt(card)
        # Graveyard cost should inject tomb-evocative language
        self.assertIn("graveyard", prompt.lower())

    def test_prompt_includes_cost_type_mill(self):
        card = {"id": "x", "name": "X", "type": "spell",
                "colors": ["blue"], "subtypes": [],
                "abilities": [], "cost": [{"amount": 2, "source": "mill"}]}
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("mill", prompt.lower())

    def test_every_prompt_has_negative(self):
        _, neg = gen_art.build_prompt(self._tusker())
        self.assertIn("photorealism", neg)

    def test_flying_ability_adds_wings_fragment(self):
        card = self._tusker()
        card["abilities"] = ["flying."]
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("wings", prompt.lower())

    def test_spell_is_bodyless(self):
        card = {"id": "x", "name": "X", "type": "instant",
                "colors": ["blue"], "subtypes": [], "abilities": []}
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("spell", prompt.lower())

    def test_symbols_inject_glyph_motif(self):
        card = self._tusker()
        card["symbols"] = ["⋈"]
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("knot", prompt.lower())

    def test_slot_keyed_symbols_inject_glyph_motif(self):
        # SLOTS.md form: symbols = { C = "꩜" }
        card = self._tusker()
        card["symbols"] = {"C": "꩜"}
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("breath", prompt.lower())

    def test_holes_inject_aesthetic_fragment(self):
        card = self._tusker()
        card["holes"] = ["C"]
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("central", prompt.lower())

    def test_face_shiny_adds_foil(self):
        card = self._tusker()
        card["face"] = ["shiny"]
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("foil", prompt.lower())


class TintRgbTests(unittest.TestCase):
    def test_single_color_returns_its_rgb(self):
        r, g, b = gen_art.tint_rgb(["red"])
        # Red channel dominant
        self.assertGreater(r, g)
        self.assertGreater(r, b)

    def test_multi_color_averages(self):
        # red ≈ (220,30,30), white ≈ (240,235,220) → midpoint
        r, g, b = gen_art.tint_rgb(["red", "white"])
        red_rgb = gen_art.COLOR_RGB["red"]
        white_rgb = gen_art.COLOR_RGB["white"]
        self.assertEqual(r, (red_rgb[0] + white_rgb[0]) // 2)

    def test_colorless_returns_none(self):
        self.assertIsNone(gen_art.tint_rgb([]))

    def test_color_rgb_covers_color_styles(self):
        # Parity: every color the prompt-builder knows about needs an RGB entry
        # so the tint can target it.
        for c in gen_art.COLOR_STYLE.keys():
            self.assertIn(c, gen_art.COLOR_RGB, f"missing RGB for {c}")


class FormatTextTests(unittest.TestCase):
    def test_cost_single(self):
        self.assertEqual(
            gen_art.format_cost([{"amount": 2, "source": "hand"}]), "2H")

    def test_cost_multiple_with_dots(self):
        self.assertEqual(
            gen_art.format_cost([
                {"amount": 2, "source": "hand"},
                {"amount": 4, "source": "graveyard"},
            ]),
            "2H · 4G")

    def test_cost_tap(self):
        self.assertEqual(gen_art.format_cost([{"source": "tap"}]), "T")

    def test_cost_x_cost(self):
        self.assertEqual(
            gen_art.format_cost([{"is_x": True, "source": "hand"}]), "XH")

    def test_cost_empty(self):
        self.assertEqual(gen_art.format_cost([]), "")

    def test_cost_unknown_source_keeps_first_letter(self):
        # Future-proof: unknown source falls back to first-letter upper.
        self.assertEqual(
            gen_art.format_cost([{"amount": 1, "source": "weirdo"}]), "1W")

    def test_type_line_with_subtypes(self):
        self.assertEqual(
            gen_art.format_type_line({
                "type": "creature", "subtypes": ["dragon"]}),
            "creature — dragon")

    def test_type_line_no_subtypes(self):
        self.assertEqual(
            gen_art.format_type_line({"type": "instant"}), "instant")

    def test_type_line_missing_type(self):
        self.assertEqual(gen_art.format_type_line({}), "")

    def test_stats_creature(self):
        self.assertEqual(
            gen_art.format_stats({"stats": {"x": 4, "y": 5}}), "4/5")

    def test_stats_missing(self):
        self.assertEqual(gen_art.format_stats({}), "")

    def test_stats_fractional(self):
        # The corpus has fractional stats (glass-* insects, etc.)
        self.assertEqual(
            gen_art.format_stats({"stats": {"x": 0.5, "y": 1}}), "0.5/1")


class TextSizeTests(unittest.TestCase):
    def test_type_line_pointsize_is_large(self):
        # User wanted "creature" (type line) bigger than the previous 12pt.
        self.assertGreaterEqual(gen_art.TYPE_POINTSIZE, 16)

    def test_abilities_pointsize_is_large(self):
        # User wanted ability text bigger than the previous 10pt.
        self.assertGreaterEqual(gen_art.ABILITIES_POINTSIZE, 14)

    def test_bottom_banner_is_tight(self):
        # User: "less large, more in line with the amount of text on a card."
        # Previous 180 was too tall; cap at 140.
        self.assertLessEqual(gen_art.BOTTOM_BANNER_H, 140)


class CardFrameTests(unittest.TestCase):
    def test_radius_is_minor(self):
        # User: "corners are slightly rounded (very minor rounding)."
        self.assertLessEqual(gen_art.FRAME_RADIUS, 16)
        self.assertGreater(gen_art.FRAME_RADIUS, 0)

    def test_border_is_thin(self):
        # User: "near black thin border."
        self.assertLessEqual(gen_art.FRAME_BORDER, 4)
        self.assertGreater(gen_art.FRAME_BORDER, 0)

    def test_border_color_near_black(self):
        # Sanity-check the hex codes to "near black."
        c = gen_art.FRAME_COLOR.lstrip("#")
        r, g, b = int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16)
        # Each channel under 32 → very dark gray, not pure black.
        self.assertLess(max(r, g, b), 32)


class BleedDimensionsTests(unittest.TestCase):
    """Generate larger than target; crop edges off so SD's self-drawn frame
    falls in the discarded margin. User: 'we own the borders, not gen_ai.'"""

    def test_15pct_bleed_dimensions(self):
        gw, gh, cx, cy = gen_art.bleed_dimensions(384, 640, 15)
        # Retain area = 1 - 2*0.15 = 0.70 → gen = target / 0.70
        # Rounded to multiples of 8.
        self.assertEqual(gw % 8, 0)
        self.assertEqual(gh % 8, 0)
        self.assertGreater(gw, 384)
        self.assertGreater(gh, 640)
        # Crop offsets must center the target in the gen canvas.
        self.assertEqual(cx, (gw - 384) // 2)
        self.assertEqual(cy, (gh - 640) // 2)

    def test_zero_bleed_is_passthrough(self):
        gw, gh, cx, cy = gen_art.bleed_dimensions(384, 640, 0)
        self.assertEqual((gw, gh, cx, cy), (384, 640, 0, 0))

    def test_15pct_bleed_matches_canonical_dimensions(self):
        # Pin the exact values so future edits don't drift unintentionally.
        gw, gh, _, _ = gen_art.bleed_dimensions(384, 640, 15)
        self.assertEqual(gw, 552)
        self.assertEqual(gh, 920)


class CostTokensTests(unittest.TestCase):
    def test_single_hand_emits_text_then_icon(self):
        toks = gen_art.cost_tokens([{"amount": 2, "source": "hand"}])
        self.assertEqual(
            toks, [("text", "2"), ("icon", gen_art.COST_ICONS["hand"])])

    def test_hand_plus_graveyard_uses_both_icons(self):
        toks = gen_art.cost_tokens([
            {"amount": 2, "source": "hand"},
            {"amount": 4, "source": "graveyard"},
        ])
        self.assertEqual(toks, [
            ("text", "2"), ("icon", gen_art.COST_ICONS["hand"]),
            ("text", " · "),
            ("text", "4"), ("icon", gen_art.COST_ICONS["graveyard"]),
        ])

    def test_tap_emits_single_text(self):
        toks = gen_art.cost_tokens([{"source": "tap"}])
        self.assertEqual(toks, [("text", "T")])

    def test_x_hand_uses_x_prefix(self):
        toks = gen_art.cost_tokens([{"is_x": True, "source": "hand"}])
        self.assertEqual(
            toks, [("text", "X"), ("icon", gen_art.COST_ICONS["hand"])])

    def test_unsupported_source_stays_letter(self):
        # Sources without an icon mapping fall back to the single-letter form.
        toks = gen_art.cost_tokens([{"amount": 1, "source": "mill"}])
        self.assertEqual(toks, [("text", "1"), ("text", "M")])

    def test_empty_cost_empty_tokens(self):
        self.assertEqual(gen_art.cost_tokens([]), [])

    def test_all_cost_icons_exist_on_disk(self):
        # Every mapped icon must actually be on disk or magick will fail.
        from pathlib import Path
        for src, path in gen_art.COST_ICONS.items():
            path_only = path.split("[")[0]
            self.assertTrue(
                Path(path_only).exists(),
                f"{src} icon missing: {path_only}")


class TextThemeTests(unittest.TestCase):
    def test_yellow_pairs_with_dark_text(self):
        theme = gen_art.text_theme(["yellow"])
        # Yellow banner with dark gray text per user spec
        self.assertEqual(theme["fg"], "#1a1a1a")

    def test_black_pairs_with_white_text(self):
        theme = gen_art.text_theme(["black"])
        self.assertEqual(theme["fg"], "white")

    def test_pink_pairs_with_black_text(self):
        theme = gen_art.text_theme(["pink"])
        self.assertEqual(theme["fg"], "black")

    def test_azure_pairs_with_dark_text(self):
        # User: "cyan with dark gray"
        theme = gen_art.text_theme(["azure"])
        self.assertEqual(theme["fg"], "#1a1a1a")

    def test_colorless_uses_default_dark_theme(self):
        theme = gen_art.text_theme([])
        self.assertIsNotNone(theme["bg"])
        self.assertIsNotNone(theme["fg"])

    def test_multi_color_uses_first_color(self):
        # User didn't pin this; first color wins as a sensible default.
        red_theme = gen_art.text_theme(["red"])
        first_red_theme = gen_art.text_theme(["red", "white"])
        self.assertEqual(red_theme["bg"], first_red_theme["bg"])

    def test_theme_coverage(self):
        # Every color the prompt-builder knows about needs a text theme.
        for c in gen_art.COLOR_STYLE.keys():
            theme = gen_art.text_theme([c])
            self.assertIn("bg", theme)
            self.assertIn("fg", theme)


class CardSeedTests(unittest.TestCase):
    def test_deterministic(self):
        self.assertEqual(gen_art.card_seed("tusker"), gen_art.card_seed("tusker"))

    def test_different_ids_different_seeds(self):
        self.assertNotEqual(gen_art.card_seed("tusker"), gen_art.card_seed("glass-moth"))


class PickTargetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)

    def test_skips_transparent_frame(self):
        cards = [
            {"id": "a", "frame": "transparent"},
            {"id": "b"},
        ]
        target = gen_art.pick_target(cards, self.tmp, seed=0)
        self.assertEqual(target["id"], "b")

    def test_skips_existing_art(self):
        Path(self.tmp, "a_384_640.png").touch()
        cards = [{"id": "a"}, {"id": "b"}]
        target = gen_art.pick_target(cards, self.tmp, seed=0)
        self.assertEqual(target["id"], "b")

    def test_deterministic_with_seed(self):
        cards = [{"id": c} for c in ["a", "b", "c", "d", "e"]]
        t1 = gen_art.pick_target(cards, self.tmp, seed=42)
        t2 = gen_art.pick_target(cards, self.tmp, seed=42)
        self.assertEqual(t1["id"], t2["id"])

    def test_none_when_all_done(self):
        for cid in ("a", "b"):
            Path(self.tmp, f"{cid}_384_640.png").touch()
        self.assertIsNone(gen_art.pick_target(
            [{"id": "a"}, {"id": "b"}], self.tmp, seed=0))


@unittest.skipUnless(shutil.which("lua5.4"), "lua5.4 not on PATH")
class LuaDriverIntegrationTests(unittest.TestCase):
    def test_loads_real_corpus(self):
        cards = gen_art.load_cards("cards")
        self.assertGreater(len(cards), 100)
        ids = {c["id"] for c in cards}
        self.assertIn("tusker", ids)
        self.assertIn("glass-moth", ids)

    def test_glass_moth_carries_holes(self):
        cards = gen_art.load_cards("cards")
        gm = next(c for c in cards if c["id"] == "glass-moth")
        self.assertEqual(set(gm["holes"]),
                         {"BR", "B", "BL", "DL", "L", "UL", "TL"})

    def test_signal_goblin_keyed_symbols_and_holes(self):
        cards = gen_art.load_cards("cards")
        sg = next(c for c in cards if c["id"] == "signal-goblin")
        # symbols = { U = "꩜" } → dict in the JSON
        self.assertEqual(sg["symbols"], {"U": "꩜"})
        self.assertEqual(sg["holes"], ["C"])


if __name__ == "__main__":
    unittest.main()
