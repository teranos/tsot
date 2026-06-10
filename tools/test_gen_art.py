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

    def test_tusker_uses_orange_style(self):
        # Orange → Indian Mughal miniature painting. The style word is what
        # makes each color visually distinct now; color enforcement moved to
        # the post-process tint.
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertIn("mughal", prompt.lower())

    def test_single_color_card_mentions_color_word_once(self):
        # One mention is enough for SD to anchor; the tint locks the actual hue.
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertGreaterEqual(prompt.lower().count("orange"), 1)

    def test_red_card_uses_aztec_or_mexican_style(self):
        card = {"id": "x", "name": "X", "type": "creature",
                "colors": ["red"], "subtypes": ["thing"], "abilities": []}
        prompt, _ = gen_art.build_prompt(card)
        self.assertTrue("aztec" in prompt.lower() or "mexican" in prompt.lower())

    def test_blue_card_uses_block_print_style(self):
        card = {"id": "x", "name": "X", "type": "creature",
                "colors": ["blue"], "subtypes": ["thing"], "abilities": []}
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("block print", prompt.lower())

    def test_colorless_card_uses_cave_painting_style(self):
        card = {"id": "x", "name": "X", "type": "creature",
                "colors": [], "subtypes": ["thing"], "abilities": []}
        prompt, _ = gen_art.build_prompt(card)
        self.assertIn("cave painting", prompt.lower())

    def test_multi_color_card_uses_first_colors_style(self):
        card = {"id": "x", "name": "X", "type": "creature",
                "colors": ["red", "green"], "subtypes": ["beast"],
                "abilities": []}
        prompt, _ = gen_art.build_prompt(card)
        # First color (red) wins the style.
        self.assertTrue("aztec" in prompt.lower() or "mexican" in prompt.lower())
        # Both colors mentioned for the tint/anchor.
        self.assertIn("red", prompt.lower())
        self.assertIn("green", prompt.lower())

    def test_every_prompt_has_composition_anchor(self):
        # Even without the heavy fauvist/ink-splatter language, every prompt
        # ends with the composition anchor so SD frames the subject.
        prompt, _ = gen_art.build_prompt(self._tusker())
        self.assertIn("subject centered", prompt.lower())

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

    def test_bottom_banner_fits_no_abilities_no_stats(self):
        # No content → very tight.
        h = gen_art.compute_bottom_banner_h("", 0, has_stats=False)
        self.assertLess(h, 30)

    def test_bottom_banner_fits_type_only(self):
        # Type line only — short banner, not the old fixed 130.
        h = gen_art.compute_bottom_banner_h("instant", 0, has_stats=False)
        self.assertGreater(h, 20)
        self.assertLess(h, 60)

    def test_bottom_banner_grows_with_abilities(self):
        short = gen_art.compute_bottom_banner_h("creature — beast", 18, has_stats=False)
        long = gen_art.compute_bottom_banner_h("creature — beast", 80, has_stats=False)
        self.assertGreater(long, short)
        # Difference matches the abilities-height delta exactly.
        self.assertEqual(long - short, 62)

    def test_bottom_banner_stats_minimum_when_otherwise_empty(self):
        # Stats badge needs vertical room even when there's no type/abilities.
        h_no_stats = gen_art.compute_bottom_banner_h("", 0, has_stats=False)
        h_with_stats = gen_art.compute_bottom_banner_h("", 0, has_stats=True)
        self.assertGreater(h_with_stats, h_no_stats)
        self.assertGreaterEqual(h_with_stats, 36)

    def test_bottom_banner_does_not_use_fixed_constant(self):
        # The old fixed BOTTOM_BANNER_H is gone; banner sizing is dynamic.
        self.assertFalse(hasattr(gen_art, "BOTTOM_BANNER_H"))


class CardFrameTests(unittest.TestCase):
    def test_radius_supports_visible_curve_past_thicker_border(self):
        # User: BOTH thicker border AND visibly rounded corners.
        # The visible curve = FRAME_RADIUS - FRAME_BORDER. With the 4px border,
        # we need at least 8px of visible curve to read as "rounded" at all.
        self.assertGreaterEqual(gen_art.FRAME_RADIUS - gen_art.FRAME_BORDER, 8)
        self.assertGreater(gen_art.FRAME_RADIUS, 0)

    def test_border_is_thicker(self):
        # User: "the current thin dark black border, make it 2x larger"
        self.assertGreaterEqual(gen_art.FRAME_BORDER, 4)

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


class MetadataTests(unittest.TestCase):
    def _kwargs(self):
        return dict(
            card={
                "id": "tusker", "name": "Tusker", "type": "creature",
                "colors": ["orange"], "subtypes": ["elephant"],
                "symbols": ["⋈"], "holes": ["C"],
            },
            seed=42,
            prompt="prompt text",
            negative="negative text",
            sd_model="/Users/s.b.vanhouten/sd-cpp/models/sd-v1-5.gguf",
            sd_lora="lcm-lora-sdv1-5",
            bleed_pct=10,
            tint_strength=25,
            watermark_active=False,
            theme_bg=(240, 130, 30),
        )

    def test_card_identity(self):
        m = gen_art.build_metadata(**self._kwargs())
        self.assertEqual(m["tsot.card.id"], "tusker")
        self.assertEqual(m["tsot.card.name"], "Tusker")
        self.assertEqual(m["tsot.card.type"], "creature")
        self.assertEqual(m["tsot.card.colors"], "orange")
        self.assertEqual(m["tsot.card.subtypes"], "elephant")
        self.assertIn("⋈", m["tsot.card.symbols"])
        self.assertEqual(m["tsot.card.holes"], "C")

    def test_generation_params(self):
        m = gen_art.build_metadata(**self._kwargs())
        self.assertEqual(m["tsot.gen.seed"], "42")
        self.assertEqual(m["tsot.gen.prompt"], "prompt text")
        self.assertEqual(m["tsot.gen.negative"], "negative text")
        # Model is recorded as just the filename, not the full path.
        self.assertEqual(m["tsot.gen.sd_model"], "sd-v1-5.gguf")
        self.assertEqual(m["tsot.gen.sd_lora"], "lcm-lora-sdv1-5")
        self.assertEqual(m["tsot.gen.steps"], "4")
        self.assertEqual(m["tsot.gen.sampler"], "lcm")

    def test_post_process_params(self):
        m = gen_art.build_metadata(**self._kwargs())
        self.assertEqual(m["tsot.post.bleed_pct"], "10")
        self.assertEqual(m["tsot.post.tint_strength"], "25")
        self.assertEqual(m["tsot.post.watermark_active"], "False")
        self.assertEqual(m["tsot.post.theme_bg"], "240,130,30")

    def test_pipeline_provenance(self):
        m = gen_art.build_metadata(**self._kwargs())
        self.assertIn("tsot.pipeline.git_commit", m)
        self.assertIn("tsot.timestamp", m)
        import re
        self.assertRegex(m["tsot.timestamp"], r"^\d{4}-\d{2}-\d{2}T")

    def test_all_values_are_strings(self):
        # magick -set property: needs string values.
        m = gen_art.build_metadata(**self._kwargs())
        for k, v in m.items():
            self.assertIsInstance(v, str, f"{k} is not a string: {type(v)}")


class SymbolPngPathTests(unittest.TestCase):
    def test_known_glyph_returns_named_path(self):
        self.assertEqual(gen_art.symbol_png_path("⋈"), "assets/icons/symbols/ax.png")
        self.assertEqual(gen_art.symbol_png_path("⨳"), "assets/icons/symbols/ix.png")
        self.assertEqual(gen_art.symbol_png_path("≡"), "assets/icons/symbols/am.png")
        self.assertEqual(gen_art.symbol_png_path("꩜"), "assets/icons/symbols/pulse.png")
        self.assertEqual(gen_art.symbol_png_path("⊨"), "assets/icons/symbols/sem.png")
        self.assertEqual(gen_art.symbol_png_path("₿"), "assets/icons/symbols/bitcoin.png")
        self.assertEqual(gen_art.symbol_png_path("Ξ"), "assets/icons/symbols/xi.png")

    def test_unknown_glyph_returns_none(self):
        self.assertIsNone(gen_art.symbol_png_path("X"))
        self.assertIsNone(gen_art.symbol_png_path(""))


class SymbolCompositeArgsTests(unittest.TestCase):
    """Title and type-line text must look uniform across every card in the
    corpus. The symbol column composite must not pull -stroke or any other
    settings that could leak into the subsequent text annotations and make
    title/type text on symbol-bearing cards render differently."""

    def test_no_stroke_setting(self):
        args = gen_art.symbol_composite_args("/tmp/test.png", x=6, y=6, pointsize=30)
        self.assertNotIn("-stroke", args)
        self.assertNotIn("-strokewidth", args)

    def test_uses_png_file_not_label(self):
        # Symbol now comes from a pre-rendered PNG, not -font label:GLYPH.
        args = gen_art.symbol_composite_args("/tmp/test.png", x=6, y=6, pointsize=30)
        self.assertNotIn("-font", args)
        self.assertNotIn("label:", " ".join(args))
        self.assertIn("/tmp/test.png", args)

    def test_uses_screen_compose(self):
        # Symbols blend with the art behind via screen compose.
        args = gen_art.symbol_composite_args("/tmp/test.png", x=6, y=6, pointsize=30)
        self.assertIn("Screen", args)


class SymbolHelperTests(unittest.TestCase):
    def test_c_slot_symbol_from_singular_field(self):
        card = {"id": "x", "symbol": "⋈"}
        self.assertEqual(gen_art.c_slot_symbol(card), "⋈")

    def test_c_slot_symbol_from_dict_form(self):
        card = {"id": "x", "symbols": {"C": "꩜", "U": "≡"}}
        self.assertEqual(gen_art.c_slot_symbol(card), "꩜")

    def test_c_slot_symbol_from_array_first_is_C(self):
        # Per SLOTS.md spiral, the first array element fills slot C.
        card = {"id": "x", "symbols": ["⨳", "⋈"]}
        self.assertEqual(gen_art.c_slot_symbol(card), "⨳")

    def test_c_slot_symbol_none_when_missing(self):
        self.assertIsNone(gen_art.c_slot_symbol({"id": "x"}))
        self.assertIsNone(gen_art.c_slot_symbol({"id": "x", "symbols": []}))

    def test_card_symbols_list_singular(self):
        card = {"id": "x", "symbol": "⋈"}
        self.assertEqual(gen_art.card_symbols_list(card), ["⋈"])

    def test_card_symbols_list_dict_spiral_order(self):
        # Dict form ordered C → U → UR → ... per SLOTS.md spiral.
        card = {"id": "x", "symbols": {"U": "A", "C": "B", "UR": "C"}}
        self.assertEqual(gen_art.card_symbols_list(card), ["B", "A", "C"])

    def test_card_symbols_list_array_preserves_order(self):
        card = {"id": "x", "symbols": ["A", "B", "C"]}
        self.assertEqual(gen_art.card_symbols_list(card), ["A", "B", "C"])

    def test_card_symbols_list_empty(self):
        self.assertEqual(gen_art.card_symbols_list({}), [])
        self.assertEqual(gen_art.card_symbols_list({"symbols": []}), [])


class WatermarkVariantTests(unittest.TestCase):
    def test_watermark_deterministic_per_card_id(self):
        # Same id → same decision every call.
        r1 = gen_art.card_uses_watermark_variant("tusker", True)
        r2 = gen_art.card_uses_watermark_variant("tusker", True)
        self.assertEqual(r1, r2)

    def test_watermark_disabled_when_no_c_symbol(self):
        # Without a C-slot symbol there's nothing to watermark with.
        self.assertFalse(gen_art.card_uses_watermark_variant("tusker", False))
        self.assertFalse(gen_art.card_uses_watermark_variant("midnight-raven", False))

    def test_watermark_rate_roughly_30pct(self):
        # Over many card ids, ~30% should land in the watermark bucket.
        # 1000 synthetic ids, expect 200-400 watermarked (loose tolerance).
        count = sum(
            1 for i in range(1000)
            if gen_art.card_uses_watermark_variant(f"card-{i}", True)
        )
        self.assertGreater(count, 200)
        self.assertLess(count, 400)

    def test_watermark_respects_custom_rate(self):
        # rate=0 → never watermark; rate=1 → always (when C-symbol present).
        self.assertFalse(gen_art.card_uses_watermark_variant("any", True, rate=0.0))
        self.assertTrue(gen_art.card_uses_watermark_variant("any", True, rate=1.0))


class CardSeedTests(unittest.TestCase):
    def test_deterministic(self):
        self.assertEqual(gen_art.card_seed("tusker"), gen_art.card_seed("tusker"))

    def test_different_ids_different_seeds(self):
        self.assertNotEqual(gen_art.card_seed("tusker"), gen_art.card_seed("glass-moth"))


class NextOutputPathTests(unittest.TestCase):
    """Never overwrite an existing PNG. First gen: {id}_{W}_{H}.png.
    Subsequent gens: append the smallest integer ≥ 2 that doesn't collide."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)

    def test_first_time_uses_base_name(self):
        p = gen_art.next_output_path(self.tmp, "tusker", 384, 640)
        self.assertEqual(p, f"{self.tmp}/tusker_384_640.png")

    def test_second_time_adds_2(self):
        Path(self.tmp, "tusker_384_640.png").touch()
        p = gen_art.next_output_path(self.tmp, "tusker", 384, 640)
        self.assertEqual(p, f"{self.tmp}/tusker_384_640_2.png")

    def test_third_time_adds_3(self):
        Path(self.tmp, "tusker_384_640.png").touch()
        Path(self.tmp, "tusker_384_640_2.png").touch()
        p = gen_art.next_output_path(self.tmp, "tusker", 384, 640)
        self.assertEqual(p, f"{self.tmp}/tusker_384_640_3.png")


class StalenessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.art = Path(self.tmp) / "art"
        self.art.mkdir()
        self.cards = Path(self.tmp) / "cards"
        self.cards.mkdir()
        self._orig_cards_dir = gen_art.CARDS_DIR
        gen_art.CARDS_DIR = str(self.cards)
        self.addCleanup(shutil.rmtree, self.tmp)
        self.addCleanup(lambda: setattr(gen_art, "CARDS_DIR", self._orig_cards_dir))

    def _lua(self, cid, mtime):
        p = self.cards / f"{cid}.lua"
        p.write_text("return {}")
        os.utime(p, (mtime, mtime))

    def _png(self, cid, mtime, w=384, h=640):
        p = self.art / f"{cid}_{w}_{h}.png"
        p.write_text("fake")
        os.utime(p, (mtime, mtime))

    def test_no_png_is_not_stale(self):
        self._lua("a", mtime=200)
        self.assertFalse(gen_art.is_stale("a", str(self.art)))

    def test_lua_newer_than_png_is_stale(self):
        self._lua("a", mtime=200)
        self._png("a", mtime=100)
        self.assertTrue(gen_art.is_stale("a", str(self.art)))

    def test_png_newer_than_lua_is_not_stale(self):
        self._lua("a", mtime=100)
        self._png("a", mtime=200)
        self.assertFalse(gen_art.is_stale("a", str(self.art)))

    def test_no_lua_is_not_stale(self):
        self._png("a", mtime=100)
        self.assertFalse(gen_art.is_stale("a", str(self.art)))


class PickTargetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.art = Path(self.tmp) / "art"
        self.art.mkdir()
        self.cards = Path(self.tmp) / "cards"
        self.cards.mkdir()
        self._orig_cards_dir = gen_art.CARDS_DIR
        gen_art.CARDS_DIR = str(self.cards)
        self.addCleanup(shutil.rmtree, self.tmp)
        self.addCleanup(lambda: setattr(gen_art, "CARDS_DIR", self._orig_cards_dir))

    def _lua(self, cid, mtime=100):
        p = self.cards / f"{cid}.lua"
        p.write_text("return {}")
        os.utime(p, (mtime, mtime))

    def _png(self, cid, mtime=100, w=384, h=640):
        p = self.art / f"{cid}_{w}_{h}.png"
        p.write_text("fake")
        os.utime(p, (mtime, mtime))

    def test_skips_transparent_frame(self):
        cards = [{"id": "a", "frame": "transparent"}, {"id": "b"}]
        target = gen_art.pick_target(cards, str(self.art), seed=0)
        self.assertEqual(target["id"], "b")

    def test_picks_fresh_when_no_stale(self):
        # 'a' has png, no lua (not stale). 'b' has no png (fresh).
        self._png("a", mtime=100)
        cards = [{"id": "a"}, {"id": "b"}]
        target = gen_art.pick_target(cards, str(self.art), seed=0)
        self.assertEqual(target["id"], "b")

    def test_deterministic_with_seed(self):
        cards = [{"id": c} for c in ["a", "b", "c", "d", "e"]]
        t1 = gen_art.pick_target(cards, str(self.art), seed=42)
        t2 = gen_art.pick_target(cards, str(self.art), seed=42)
        self.assertEqual(t1["id"], t2["id"])

    def test_stale_card_wins_over_fresh(self):
        # 'a' is stale (lua newer than png), 'b' is fresh (no png).
        # Stale must win even though fresh is normally picked.
        self._lua("a", mtime=200)
        self._png("a", mtime=100)
        cards = [{"id": "a"}, {"id": "b"}]
        target = gen_art.pick_target(cards, str(self.art), seed=0)
        self.assertEqual(target["id"], "a")

    def test_stale_card_wins_over_complete_corpus(self):
        # Every card has a png; one is stale.
        self._lua("a", mtime=100); self._png("a", mtime=200)  # not stale
        self._lua("b", mtime=300); self._png("b", mtime=100)  # stale
        self._lua("c", mtime=50); self._png("c", mtime=200)   # not stale
        cards = [{"id": "a"}, {"id": "b"}, {"id": "c"}]
        target = gen_art.pick_target(cards, str(self.art), seed=0)
        self.assertEqual(target["id"], "b")


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
