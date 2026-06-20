#!/usr/bin/env python3
"""tsot — card rendering layer.

Decorates an interior PNG with the TSOT card shell: themed banners, name,
cost, type line, abilities, stats badge, symbol column (or watermark
variant), holes, color tint, rounded-corner frame.

Knows nothing about HOW the interior PNG was produced (SD, hand-drawn,
hosted API, etc.). Given a PNG of the canonical card canvas size
(WIDTH × HEIGHT) plus the card's .lua data, it shells out to `magick`
and writes the decorated card back in place.
"""
from __future__ import annotations

import hashlib
import os
import subprocess
from pathlib import Path


# --- Canvas -------------------------------------------------------------

WIDTH = 384
HEIGHT = 640


# --- Slot grid (SLOTS.md) ----------------------------------------------

SLOT_W = WIDTH // 3
SLOT_H = HEIGHT // 5
SLOT_GRID = [
    ["TL", "T", "TR"],
    ["UL", "U", "UR"],
    ["L",  "C", "R"],
    ["DL", "D", "DR"],
    ["BL", "B", "BR"],
]

# Default spiral order when symbols are declared as an array (not slot-keyed).
# First array element lands at slot C.
SLOT_SPIRAL = ["C", "U", "UR", "R", "DR", "D", "DL", "L", "UL",
               "TL", "T", "TR", "BR", "B", "BL"]


def slot_rect(slot: str) -> tuple[int, int, int, int]:
    for row_idx, row in enumerate(SLOT_GRID):
        for col_idx, name in enumerate(row):
            if name == slot:
                return (col_idx * SLOT_W, row_idx * SLOT_H, SLOT_W, SLOT_H)
    raise ValueError(f"unknown slot: {slot}")


# --- Color theming ------------------------------------------------------

# RGB targets for the post-process tint.
COLOR_RGB = {
    "red":    (220, 30, 30),
    "blue":   (30, 80, 220),
    "black":  (30, 20, 30),
    "white":  (240, 235, 220),
    "green":  (40, 160, 60),
    "pink":   (255, 130, 200),
    "purple": (140, 60, 200),
    "azure":  (90, 200, 220),
    "orange": (240, 130, 30),
    "yellow": (240, 220, 30),
    "brown":  (140, 80, 30),
}

# Per-color text-overlay theme. User-pinned pairings — yellow/azure/orange/
# pink/white land on light banners that need dark text; everything else lands
# on dark/saturated banners that need white text.
COLOR_TEXT_THEME = {
    "red":    {"bg": (220, 30, 30),   "fg": "white"},
    "blue":   {"bg": (30, 80, 220),   "fg": "white"},
    "black":  {"bg": (20, 20, 25),    "fg": "white"},
    "white":  {"bg": (240, 235, 220), "fg": "#1a1a1a"},
    "green":  {"bg": (40, 160, 60),   "fg": "white"},
    "pink":   {"bg": (255, 130, 200), "fg": "black"},
    "purple": {"bg": (140, 60, 200),  "fg": "white"},
    "azure":  {"bg": (90, 200, 220),  "fg": "#1a1a1a"},
    "orange": {"bg": (240, 130, 30),  "fg": "#1a1a1a"},
    "yellow": {"bg": (240, 220, 30),  "fg": "#1a1a1a"},
    "brown":  {"bg": (140, 80, 30),   "fg": "white"},
}
DEFAULT_TEXT_THEME = {"bg": (40, 40, 45), "fg": "white"}


def text_theme(colors: list[str]) -> dict:
    """Pick banner+text colors for overlay. First card color wins; empty → default."""
    for c in colors:
        if c in COLOR_TEXT_THEME:
            return COLOR_TEXT_THEME[c]
    return DEFAULT_TEXT_THEME


# --- Text formatters (card data → display strings) ---------------------

COST_SOURCE_SYMBOLS = {
    "hand": "H", "graveyard": "G", "mill": "M",
    "sacrifice": "S", "self": "X", "attached": "A",
}

# Cost-source → icon file. Sources here render as composited icons in the
# cost line; sources missing here fall back to the single-letter text form.
COST_ICONS = {
    "hand":      "assets/icons/hand.ico[0]",
    "graveyard": "assets/icons/graveyard.jpg",
}


def cost_tokens(cost: list[dict]) -> list[tuple[str, str]]:
    """Decompose a cost list into a sequence of (kind, value) tokens for
    the icon-aware renderer. kind is 'text' or 'icon'."""
    out: list[tuple[str, str]] = []
    for i, c in enumerate(cost or []):
        if i > 0:
            out.append(("text", " · "))
        src = (c or {}).get("source") or ""
        amt = (c or {}).get("amount")
        is_x = (c or {}).get("is_x", False)
        if src == "tap":
            out.append(("text", "T"))
            continue
        if is_x:
            out.append(("text", "X"))
        elif amt is not None:
            out.append(("text", str(amt)))
        icon = COST_ICONS.get(src)
        if icon:
            out.append(("icon", icon))
        else:
            letter = COST_SOURCE_SYMBOLS.get(src) or (src[:1].upper() if src else "?")
            out.append(("text", letter))
    return out


def format_cost(cost: list[dict]) -> str:
    parts: list[str] = []
    for c in cost or []:
        src = (c or {}).get("source") or ""
        amt = (c or {}).get("amount")
        is_x = (c or {}).get("is_x", False)
        if src == "tap":
            parts.append("T")
        elif is_x:
            sym = COST_SOURCE_SYMBOLS.get(src) or (src[:1].upper() if src else "?")
            parts.append(f"X{sym}")
        else:
            sym = COST_SOURCE_SYMBOLS.get(src) or (src[:1].upper() if src else "?")
            parts.append(f"{amt}{sym}")
    return " · ".join(parts)


def format_type_line(card: dict) -> str:
    typ = card.get("type") or ""
    if not typ:
        return ""
    subtypes = card.get("subtypes") or []
    if subtypes:
        return f"{typ} — {', '.join(subtypes)}"
    return typ


def format_stats(card: dict) -> str:
    s = card.get("stats") or {}
    x, y = s.get("x"), s.get("y")
    if x is None or y is None:
        return ""

    def fmt(v):
        return str(int(v)) if isinstance(v, (int, float)) and float(v).is_integer() else str(v)
    return f"{fmt(x)}/{fmt(y)}"


# --- Layout constants (overlay_text) -----------------------------------

TOP_BANNER_H = 60
BANNER_ALPHA = 0.85
NAME_POINTSIZE = 22
COST_POINTSIZE = 14
TYPE_POINTSIZE = 18
ABILITIES_POINTSIZE = 14
STATS_POINTSIZE = 24
SYMBOL_POINTSIZE = 30
SYMBOL_LEFT_PAD = 6
SYMBOL_TOP_PAD = 6
SYMBOL_GAP = 4
WATERMARK_POINTSIZE = 110
WATERMARK_ALPHA = 0.18
WATERMARK_RATE = 0.30
FONT_BOLD = "Helvetica-Bold"
FONT_REG = "Helvetica"


# --- Symbol helpers ----------------------------------------------------

def c_slot_symbol(card: dict) -> str | None:
    """Return the symbol glyph at slot C, or None if the card has no C-slot symbol."""
    if card.get("symbol"):
        return card["symbol"]
    raw = card.get("symbols")
    if not raw:
        return None
    if isinstance(raw, dict):
        return raw.get("C")
    if isinstance(raw, list) and len(raw) > 0:
        return raw[0]
    return None


def card_symbols_list(card: dict) -> list[str]:
    """All symbols on a card in column order (C first, then surrounding slots)."""
    if card.get("symbol"):
        return [card["symbol"]]
    raw = card.get("symbols")
    if not raw:
        return []
    if isinstance(raw, dict):
        return [raw[slot] for slot in SLOT_SPIRAL if slot in raw]
    if isinstance(raw, list):
        return list(raw)
    return []


def card_uses_watermark_variant(card_id: str, has_c_symbol: bool,
                                 rate: float = WATERMARK_RATE) -> bool:
    """Deterministic per-card decision: should this card render with the
    watermark variant instead of the symbol column?"""
    if not has_c_symbol:
        return False
    h = hashlib.sha256(f"watermark:{card_id}".encode()).digest()
    val = int.from_bytes(h[:8], "big") / float(1 << 64)
    return val < rate


# Corpus symbol glyph → simple name. Pre-rendered PNGs live at
# assets/icons/symbols/{name}.png (gitignored).
SYMBOL_GLYPH_TO_NAME = {
    "⋈": "ax",
    "⨳": "ix",
    "≡": "am",
    "꩜": "pulse",
    "⊨": "sem",
    "₿": "bitcoin",
    "Ξ": "xi",
}

SYMBOLS_DIR = "assets/icons/symbols"


def symbol_png_path(glyph: str) -> str | None:
    """Return the PNG path for a known corpus symbol glyph, or None."""
    name = SYMBOL_GLYPH_TO_NAME.get(glyph)
    if name is None:
        return None
    return f"{SYMBOLS_DIR}/{name}.png"


def symbol_composite_args(png_path: str, x: int, y: int, pointsize: int) -> list[str]:
    """Build the magick args to composite ONE pre-rendered symbol PNG onto
    the art. Resized to the target pointsize-equivalent height, screen-
    blended for art integration. No -stroke — stroke would leak into
    subsequent text annotations and break title/type uniformity."""
    return [
        "(",
        png_path, "-resize", f"x{pointsize}",
        ")",
        "-gravity", "northwest", "-geometry", f"+{x}+{y}",
        "-compose", "Screen", "-composite",
        "-compose", "Over",
    ]


def compute_bottom_banner_h(type_line: str, abilities_image_height: int,
                             has_stats: bool) -> int:
    """Height of the bottom banner that fits its content tightly."""
    BOT_PAD = 10
    TOP_PAD = 8
    GAP = 8
    TYPE_H = 22
    STATS_MIN = 38
    h = BOT_PAD
    if abilities_image_height > 0:
        h += abilities_image_height
    if abilities_image_height > 0 and type_line:
        h += GAP
    if type_line:
        h += TYPE_H
    h += TOP_PAD
    if has_stats:
        h = max(h, STATS_MIN)
    return h


def render_cost_strip(tokens: list[tuple[str, str]], font: str,
                      pointsize: int, fg: str, icon_h: int, out_path: str) -> bool:
    """Build a horizontal PNG strip of cost tokens. Returns False if empty."""
    if not tokens:
        return False
    cmd = ["magick"]
    for kind, value in tokens:
        if kind == "text":
            cmd += [
                "(",
                "-background", "none", "-fill", fg, "-font", font,
                "-pointsize", str(pointsize),
                f"label:{value}",
                ")",
            ]
        else:  # icon
            cmd += [
                "(",
                value,
                "-resize", f"x{icon_h}",
                "-fuzz", "8%", "-transparent", "white",
                "-channel", "RGB", "+level-colors", f"{fg},{fg}", "+channel",
                ")",
            ]
    cmd += ["-background", "none", "-gravity", "Center", "+append", out_path]
    subprocess.run(cmd, check=True)
    return True


def overlay_text(png_path: str, card: dict) -> None:
    theme = text_theme(card.get("colors") or [])
    r, g, b = theme["bg"]
    bg_rgba = f"rgba({r},{g},{b},{BANNER_ALPHA})"
    fg = theme["fg"]

    name = card.get("name") or card.get("id", "")
    cost_raw = card.get("cost") or []
    type_line = format_type_line(card)
    abilities = card.get("abilities") or []
    abilities_text = ". ".join(a.rstrip(".") for a in abilities if a) + ("." if abilities else "")
    stats = format_stats(card)

    symbols = card_symbols_list(card)
    c_sym = c_slot_symbol(card)
    use_watermark = card_uses_watermark_variant(card.get("id", ""), c_sym is not None)

    abilities_strip = None
    abilities_h = 0
    if abilities_text:
        wrap_w = WIDTH - 70
        abilities_strip = f"/tmp/abilities-{os.getpid()}.png"
        subprocess.run([
            "magick",
            "-background", "none", "-fill", fg, "-font", FONT_REG,
            "-pointsize", str(ABILITIES_POINTSIZE),
            "-size", f"{wrap_w}x200",
            f"caption:{abilities_text}",
            "-trim", "+repage",
            abilities_strip,
        ], check=True)
        abilities_h = int(subprocess.check_output(
            ["magick", "identify", "-format", "%h", abilities_strip],
            text=True,
        ).strip())

    bottom_banner_h = compute_bottom_banner_h(type_line, abilities_h, bool(stats))

    cmd = [
        "magick", png_path,
        "(", "-size", f"{WIDTH}x{TOP_BANNER_H}", f"xc:{bg_rgba}", ")",
        "-gravity", "north", "-composite",
        "(", "-size", f"{WIDTH}x{bottom_banner_h}", f"xc:{bg_rgba}", ")",
        "-gravity", "south", "-composite",
    ]

    if use_watermark and c_sym:
        wm_y = max(4, (bottom_banner_h - WATERMARK_POINTSIZE) // 2)
        cmd += [
            "(",
            "-background", "none", "-fill", fg, "-font", FONT_BOLD,
            "-pointsize", str(WATERMARK_POINTSIZE),
            f"label:{c_sym}",
            "-alpha", "set",
            "-channel", "A", "-evaluate", "Multiply", str(WATERMARK_ALPHA), "+channel",
            ")",
            "-gravity", "south", "-geometry", f"+0+{wm_y}", "-composite",
        ]
    elif symbols:
        y = SYMBOL_TOP_PAD
        for sym in symbols:
            png = symbol_png_path(sym)
            if png and Path(png).exists():
                cmd += symbol_composite_args(png, SYMBOL_LEFT_PAD, y, SYMBOL_POINTSIZE)
                y += SYMBOL_POINTSIZE + SYMBOL_GAP

    name_x = (SYMBOL_LEFT_PAD + SYMBOL_POINTSIZE + 6) if (symbols and not use_watermark) else 10
    cmd += [
        "-gravity", "northwest", "-font", FONT_REG,
        "-pointsize", str(NAME_POINTSIZE), "-fill", fg,
        "-annotate", f"+{name_x}+12", name,
    ]

    if cost_raw:
        tokens = cost_tokens(cost_raw)
        strip_path = f"/tmp/cost-strip-{os.getpid()}.png"
        if render_cost_strip(tokens, FONT_REG, COST_POINTSIZE, fg,
                             COST_POINTSIZE + 4, strip_path):
            cmd += [
                "(", strip_path, ")",
                "-gravity", "northeast", "-geometry", "+10+14", "-composite",
            ]

    if type_line:
        BOT_PAD = 10
        GAP = 8 if abilities_h > 0 else 0
        type_y = BOT_PAD + abilities_h + GAP + 4
        cmd += [
            "-gravity", "southwest", "-font", FONT_REG,
            "-pointsize", str(TYPE_POINTSIZE), "-fill", fg,
            "-annotate", f"+10+{type_y}", type_line,
        ]

    if abilities_strip:
        cmd += [
            "(", abilities_strip, ")",
            "-gravity", "southwest", "-geometry", "+10+10", "-composite",
        ]

    if stats:
        cmd += [
            "-gravity", "southeast", "-font", FONT_REG,
            "-pointsize", str(STATS_POINTSIZE), "-fill", fg,
            "-annotate", "+8+6", stats,
        ]

    cmd += [png_path]
    subprocess.run(cmd, check=True)


# --- Color tint --------------------------------------------------------

def tint_rgb(colors: list[str]) -> tuple[int, int, int] | None:
    """Compute target RGB for the post-process tint. None = no tint (colorless)."""
    known = [c for c in colors if c in COLOR_RGB]
    if not known:
        return None
    rs, gs, bs = zip(*(COLOR_RGB[c] for c in known))
    return (sum(rs) // len(rs), sum(gs) // len(gs), sum(bs) // len(bs))


def tint_to_card_color(png_path: str, colors: list[str], strength: int) -> None:
    """Pull the generated image toward the card's color average."""
    if strength <= 0:
        return
    rgb = tint_rgb(colors)
    if rgb is None:
        return
    r, g, b = rgb
    subprocess.run([
        "magick", png_path,
        "-fill", f"rgb({r},{g},{b})",
        "-colorize", str(strength),
        "-modulate", "100,140,100",
        png_path,
    ], check=True)


# --- Frame -------------------------------------------------------------

FRAME_RADIUS = 14
FRAME_BORDER = 4
FRAME_COLOR = "#0a0a0a"


def apply_card_frame(png_path: str) -> None:
    w, h = WIDTH, HEIGHT
    r = FRAME_RADIUS
    stroke_w = FRAME_BORDER * 2
    subprocess.run([
        "magick", png_path,
        "-fill", "none",
        "-stroke", FRAME_COLOR,
        "-strokewidth", str(stroke_w),
        "-draw", f"roundRectangle 0,0 {w - 1},{h - 1} {r},{r}",
        "(",
        "-size", f"{w}x{h}", "xc:none",
        "-fill", "white",
        "-draw", f"roundRectangle 0,0 {w - 1},{h - 1} {r},{r}",
        ")",
        "-compose", "DstIn", "-composite",
        png_path,
    ], check=True)


# --- Hole carving ------------------------------------------------------

def carve_holes(png_path: str, holes: list[str]) -> None:
    args = ["magick", png_path, "-alpha", "set"]
    for slot in holes:
        x, y, w, h = slot_rect(slot)
        args += ["-region", f"{w}x{h}+{x}+{y}",
                 "-channel", "A", "-evaluate", "set", "0"]
    args += ["+channel", png_path]
    subprocess.run(args, check=True)
