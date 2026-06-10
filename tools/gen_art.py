#!/usr/bin/env python3
"""tsot — generate a piece of art for one random card that doesn't have it yet.

Invoked by `make art`. Picks a card whose `gen_art/{id}_*.png` is missing,
builds a prompt from its .lua metadata + the global style suffix, shells
out to `sd` (stable-diffusion.cpp) for the image, then carves slot-grid
holes via `magick` if the card declares any.

Env (all optional — defaults match the install at ~/sd-cpp/):
    SD_BIN     path to the sd-cli binary (default: ~/sd-cpp/stable-diffusion.cpp/build/bin/sd-cli)
    SD_MODEL   path to the SD 1.5 base weights (default: ~/sd-cpp/models/v1-5-pruned-emaonly.safetensors)
    SD_LORA    LCM LoRA filename stem (default: "lcm-lora-sdv1-5"); script appends
               <lora:$SD_LORA:1> to every prompt and uses 4-step LCM sampling
    SD_LORA_DIR  directory containing the LoRA file (default: dirname of SD_MODEL)

    Throttle knobs — defaults keep the machine usable while sd runs.
    SD_THREADS         CPU threads sd-cli may use (default: 2 — leaves most cores for the OS)
    SD_VRAM_RESERVE    GiB of VRAM to keep free for the system (default: 3.0)
    SD_TAESD           path to taesd_decoder.safetensors (auto-detected at ~/sd-cpp/models/)
                       Cuts VAE decode from ~8s to ~1s with small quality drop on details.
    SD_FAST=1          disable throttling: use all cores, no VRAM reservation (faster, machine unusable)

Output: gen_art/{id}_{W}_{H}.png
"""
from __future__ import annotations

import hashlib
import json
import os
import random
import subprocess
import sys
from pathlib import Path


CARDS_DIR = "cards"
ART_DIR = "gen_art"
WIDTH = 384
HEIGHT = 640

# Local install paths produced by the one-time setup (see `make art` help text).
# Override with SD_BIN / SD_MODEL env vars if you've installed elsewhere.
DEFAULT_SD_BIN = str(Path.home() / "sd-cpp" / "stable-diffusion.cpp" / "build" / "bin" / "sd-cli")
DEFAULT_SD_MODEL = str(Path.home() / "sd-cpp" / "models" / "v1-5-pruned-emaonly-q5_1.gguf")
# Combined TAESD (encoder + decoder) — the decoder-only file from
# madebyollin/taesd is missing tensors that stable-diffusion.cpp expects
# (probed 2026-06-10). Use the diffusion_pytorch_model.safetensors version.
DEFAULT_SD_TAESD = str(Path.home() / "sd-cpp" / "models" / "taesd.safetensors")


def bleed_dimensions(target_w: int, target_h: int, bleed_pct: int) -> tuple[int, int, int, int]:
    """Generate at (gen_w, gen_h), crop centered to (target_w, target_h).

    bleed_pct is the percentage cut off EACH side of the generated image.
    SD reliably draws a self-imposed frame at the canvas edges; pushing the
    real art inward, then cropping the frame off, gives true full-bleed
    output. Returns (gen_w, gen_h, crop_x, crop_y).
    """
    if bleed_pct <= 0:
        return (target_w, target_h, 0, 0)
    keep = 1.0 - 2.0 * (bleed_pct / 100.0)

    def round_up_8(x: float) -> int:
        return ((int(x) + 7) // 8 + (0 if x == int(x) else 1)) * 8 if x != int(x) else ((int(x) + 7) // 8) * 8

    # Simpler: ceil to next multiple of 8.
    import math
    gw = int(math.ceil(target_w / keep / 8.0)) * 8
    gh = int(math.ceil(target_h / keep / 8.0)) * 8
    return (gw, gh, (gw - target_w) // 2, (gh - target_h) // 2)

# 5 rows × 3 cols per SLOTS.md. 128 px per slot at 384 × 640.
SLOT_W = WIDTH // 3
SLOT_H = HEIGHT // 5
SLOT_GRID = [
    ["TL", "T", "TR"],
    ["UL", "U", "UR"],
    ["L",  "C", "R"],
    ["DL", "D", "DR"],
    ["BL", "B", "BR"],
]


def slot_rect(slot: str) -> tuple[int, int, int, int]:
    for row_idx, row in enumerate(SLOT_GRID):
        for col_idx, name in enumerate(row):
            if name == slot:
                return (col_idx * SLOT_W, row_idx * SLOT_H, SLOT_W, SLOT_H)
    raise ValueError(f"unknown slot: {slot}")


# ---------------------------------------------------------------------
# Card loading via lua5.4 subprocess (same pattern as cards-report.py).
# ---------------------------------------------------------------------

LUA_DRIVER = r"""
local dir = arg[1]
local function escape_str(s)
  s = s:gsub("\\", "\\\\"):gsub('"', '\\"')
       :gsub("\b", "\\b"):gsub("\f", "\\f"):gsub("\n", "\\n")
       :gsub("\r", "\\r"):gsub("\t", "\\t")
  return s:gsub("[%z\1-\31]", function(c) return string.format("\\u%04x", c:byte()) end)
end
local function is_array(t)
  local n = 0
  for k, _ in pairs(t) do
    if type(k) ~= "number" then return false end
    if k > n then n = k end
  end
  for i = 1, n do if t[i] == nil then return false end end
  return true, n
end
local function encode(v)
  local tv = type(v)
  if tv == "string" then return '"' .. escape_str(v) .. '"'
  elseif tv == "number" then
    if v ~= v or v == math.huge or v == -math.huge then return "null" end
    if v == math.floor(v) and math.abs(v) < 1e15 then
      return tostring(math.floor(v))
    end
    return tostring(v)
  elseif tv == "boolean" then return v and "true" or "false"
  elseif tv == "nil" then return "null"
  elseif tv == "table" then
    local ok, n = is_array(v)
    if ok then
      local parts = {}
      for i = 1, n do parts[i] = encode(v[i]) end
      return "[" .. table.concat(parts, ",") .. "]"
    else
      local parts = {}
      local keys = {}
      for k, _ in pairs(v) do
        if type(k) == "string" then keys[#keys + 1] = k end
      end
      table.sort(keys)
      for _, k in ipairs(keys) do
        local val = v[k]
        local tval = type(val)
        if tval ~= "function" and tval ~= "userdata" and tval ~= "thread" then
          parts[#parts + 1] = '"' .. escape_str(k) .. '":' .. encode(val)
        end
      end
      return "{" .. table.concat(parts, ",") .. "}"
    end
  else
    return "null"
  end
end

local files = {}
local p = io.popen("ls " .. dir .. "/*.lua 2>/dev/null")
if p then
  for line in p:lines() do files[#files + 1] = line end
  p:close()
end
table.sort(files)

for _, path in ipairs(files) do
  local ok, result = pcall(dofile, path)
  if not ok then
    io.stderr:write("ERR\t" .. path .. "\t" .. tostring(result) .. "\n")
  elseif type(result) ~= "table" then
    io.stderr:write("ERR\t" .. path .. "\tdid not return a table\n")
  else
    io.write(encode(result), "\n")
  end
end
"""


def load_cards(cards_dir: str) -> list[dict]:
    proc = subprocess.run(
        ["lua5.4", "-", cards_dir],
        input=LUA_DRIVER, capture_output=True, text=True, check=True,
    )
    return [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]


# ---------------------------------------------------------------------
# Prompt construction.
# ---------------------------------------------------------------------

# Anchor language drives the overall composition. Dropped "Trading Card Game"
# — those tokens reliably make SD 1.5 draw a literal frame inside the art
# (the opponent-draw / unblockable-human inner-frame bug). "Full-bleed" +
# repeated edge-to-edge language counters the frame prior.
TCG_ANCHOR = (
    "Full-bleed fantasy illustration, edge-to-edge composition, "
    "art fills the entire canvas, no inner border, no frame around the art, "
    "subject bled to the corners"
)

# Lightened — the per-color COLOR_STYLE now carries the visual idiom, and
# this just nudges SD on composition + against photorealism.
STYLE_SUFFIX = (
    "no photorealism, vertical portrait composition with subject centered"
)

TYPE_PHRASES = {
    "creature":    "creature illustration",
    "instant":     "instant spell mid-cast, no body, abstract effect",
    "sorcery":     "sorcery spell, ritual diagram",
    "spell":       "spell mid-cast, abstract effect",
    "artifact":    "magical artifact",
    "environment": "magical environment landscape",
    "mutation":    "creature mutation, attached parasite or modification",
    "symbol":      "magical glyph card, single dominant symbol",
}

# Cost source → evocative imagery hint. The card's cost-payment mechanics
# carry thematic weight (graveyard = death-touched, mill = burned knowledge,
# sacrifice = blood). Surfacing these in the prompt anchors the art in the
# card's *flavor*, not just its color.
COST_PHRASES = {
    "hand":      "summoned from the hand",
    "graveyard": "raised from the graveyard, tomb-touched, bones and ash",
    "mill":      "milled from the deck top, burned pages, knowledge spent",
    "attached":  "wearing previous incarnations, mutation-layered",
    "sacrifice": "blood-cost paid, sacrificial imagery",
    "self":      "self-exiled, exile-bound",
    "tap":       "exertion, depletion",
}
NEGATIVE = (
    "photorealism, realistic, 3d render, smooth shading, muted, "
    "desaturated, soft pastel, photograph, text, watermark, signature, "
    "anatomically correct, technical illustration, "
    # Multi-color enemies — push SD away from rainbow output so the per-card
    # color anchor in the prompt has room to win.
    "rainbow, multicolor chaos, every color, prism, color wheel, "
    # Inner-border bug: SD reads card-language as "draw a card frame in the
    # art." Pile on every framing word so the prior gets crushed.
    "card frame, inner border, outer border, picture frame, ornamental border, "
    "framed illustration, mat board, white frame, white border, beveled edge, "
    "art inside a frame, double border, vignette, border decoration, "
    "rectangular frame, drawn outline frame, art behind glass"
)

# Per-color art tradition. The IDIOM carries the visual identity of each
# color — instead of every card looking like a color-locked ink splatter,
# red cards look like Aztec codex pages, blue cards like block prints,
# white cards like Roman frescoes. Post-process tint (default 25%) then
# locks the actual hue.
COLOR_STYLE = {
    "red":    "Aztec codex page illumination, Mexican muralism in the style of Diego Rivera, bold flat warm tones",
    "blue":   "linocut block print, bold carved lines, ink on paper, high contrast",
    "green":  "woodcut print, German Expressionist black-line carving, bold gestural cuts",
    "white":  "Roman fresco, ancient wall painting, weathered plaster, classical figures",
    "black":  "gothic woodcut, Aubrey Beardsley ink illustration, high-contrast black and white",
    "pink":   "art nouveau poster, Alphonse Mucha decorative illustration, flowing organic lines",
    "purple": "psychedelic concert poster, 1960s op-art, swirling distortion",
    "azure":  "cathedral stained glass window, leaded glass illumination, jewel-toned light",
    "orange": "Indian Mughal miniature painting, ornate detail, gold-leaf embellishment",
    "yellow": "medieval illuminated manuscript, gold-leaf illumination, decorated initial",
    "brown":  "cave painting, ochre and umber on stone, primal handprints",
}

# Colorless cards (Scavenger Rat, Pale Apparition, the Clear cycle) get the
# primal/unbranded look — fits the "no color identity" theme.
NEUTRAL_STYLE = "cave painting, ochre and charcoal on stone, primitive primal mark-making"

# RGB targets for the post-process tint. Saturated mid-luminance values so the
# magick -tint operator (which biases mid-tones toward the fill) lands in the
# perceptually-correct color without crushing shadows or highlights.
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

GLYPH_MOTIFS = {
    "⋈": "knot-glyph motif woven through the composition",
    "⨳": "sealed-cross glyph motif",
    "≡": "triple-parallel-stripe motif",
    "꩜": "breath-spiral glyph",
    "⊨": "turnstile glyph motif",
    "₿": "bitcoin-glyph motif",
    "Ξ": "tri-bar Ξ-glyph motif",
}


def _normalize_symbols(raw) -> list[str]:
    """Accepts list (array form) or dict (slot-keyed form per SLOTS.md)."""
    if not raw:
        return []
    if isinstance(raw, dict):
        return list(raw.values())
    if isinstance(raw, list):
        return raw
    return []


def holes_aesthetic(holes: list[str]) -> str:
    if not holes:
        return ""
    s = set(holes)
    left_col = {"TL", "UL", "L", "DL", "BL"}
    right_col = {"TR", "UR", "R", "DR", "BR"}
    if left_col.issubset(s) and right_col.issubset(s):
        return ("vertical wraith body, dissolving at left and right edges, "
                "central spine intact")
    if len(s) >= 7:
        return "glass body, translucent chitin, mostly air, ragged voids"
    if "C" in s and len(s) == 1:
        return "composition framing a single glowing central void"
    if len(s) >= 4:
        return "fragmented body, holes blown through, ink-bled edges"
    return "punched-out negative space at the card's edges"


def build_prompt(card: dict) -> tuple[str, str]:
    parts: list[str] = [TCG_ANCHOR]

    name = card.get("name") or card.get("id", "")
    if name:
        parts.append(name)

    typ = card.get("type", "")
    type_phrase = TYPE_PHRASES.get(typ)
    if type_phrase:
        parts.append(type_phrase)

    subtypes = card.get("subtypes") or []
    if subtypes:
        parts.append(", ".join(subtypes))

    cost = card.get("cost") or []
    cost_seen: set[str] = set()
    for c in cost:
        src = (c or {}).get("source")
        if src and src not in cost_seen and src in COST_PHRASES:
            parts.append(COST_PHRASES[src])
            cost_seen.add(src)

    card_colors = card.get("colors") or []
    if card_colors:
        # First color picks the visual idiom; all colors named once for the
        # color anchor + downstream post-process tint.
        primary = card_colors[0]
        style = COLOR_STYLE.get(primary)
        if style:
            parts.append(style)
        color_phrase = " and ".join(card_colors) + " tones"
        parts.append(f"in {color_phrase}")
    else:
        parts.append(NEUTRAL_STYLE)

    for glyph in _normalize_symbols(card.get("symbols")):
        motif = GLYPH_MOTIFS.get(glyph)
        if motif:
            parts.append(motif)

    face = card.get("face") or []
    if "shiny" in face:
        parts.append("holographic foil sheen")
    if "glow" in face:
        parts.append("self-emissive, bloomy halo")

    holes_frag = holes_aesthetic(card.get("holes") or [])
    if holes_frag:
        parts.append(holes_frag)

    abilities_str = " ".join(card.get("abilities") or [])
    if "flying" in abilities_str:
        parts.append("wings spread, airborne")
    if "haste" in abilities_str:
        parts.append("motion-blurred, charging")
    if "defender" in abilities_str:
        parts.append("planted, defensive stance")

    parts.append(STYLE_SUFFIX)
    return ", ".join(p for p in parts if p), NEGATIVE


# ---------------------------------------------------------------------
# Target selection & seeding.
# ---------------------------------------------------------------------

def pick_target(cards: list[dict], art_dir: str, seed: int) -> dict | None:
    todo: list[dict] = []
    for c in cards:
        if c.get("frame") == "transparent":
            continue
        if list(Path(art_dir).glob(f"{c['id']}_*.png")):
            continue
        todo.append(c)
    if not todo:
        return None
    rng = random.Random(seed)
    return rng.choice(sorted(todo, key=lambda c: c["id"]))


def card_seed(card_id: str) -> int:
    return int(hashlib.sha256(card_id.encode()).hexdigest()[:8], 16)


# Per-color text-overlay theme. User-pinned pairings — yellow/azure/orange/
# pink/white land on light banners that need dark text; everything else lands
# on dark/saturated banners that need white text. RGB matches COLOR_RGB.
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


# ---------------------------------------------------------------------
# Text formatters — pure transforms from card data → display strings.
# ---------------------------------------------------------------------

COST_SOURCE_SYMBOLS = {
    "hand": "H", "graveyard": "G", "mill": "M",
    "sacrifice": "S", "self": "X", "attached": "A",
}

# Cost-source → icon file. Sources here render as composited icons in the
# cost line; sources missing here fall back to the single-letter text form
# (M for mill, S for sacrifice, etc.). Add a file under assets/icons/ and
# wire it in here to upgrade another source. [0] on the ICO picks the
# 256x256 PNG frame for highest-quality resize.
COST_ICONS = {
    "hand":      "assets/icons/hand.ico[0]",
    "graveyard": "assets/icons/graveyard.jpg",
}


def cost_tokens(cost: list[dict]) -> list[tuple[str, str]]:
    """Decompose a cost list into a sequence of (kind, value) tokens for
    the icon-aware renderer. kind is 'text' or 'icon'. Cost sources with an
    icon emit ('icon', path); the rest emit ('text', single-letter)."""
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
        # Fractional stats survived from the corpus (glass-* insects).
        return str(int(v)) if isinstance(v, (int, float)) and float(v).is_integer() else str(v)
    return f"{fmt(x)}/{fmt(y)}"


# ---------------------------------------------------------------------
# Text overlay via ImageMagick. Paints two solid-color banners (top for
# name + cost, bottom for type line + abilities + stats badge) in the
# card's themed color, then draws the text in the matched contrast color.
# Runs before carve_holes so holes still punch through any banner that
# happens to overlap a hole slot.
# ---------------------------------------------------------------------

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


# Per SLOTS.md, the default spiral order when a card declares symbols as an
# array (not slot-keyed). The first array element lands at slot C.
SLOT_SPIRAL = ["C", "U", "UR", "R", "DR", "D", "DL", "L", "UL",
               "TL", "T", "TR", "BR", "B", "BL"]


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
    watermark variant instead of the symbol column? Hash the id so the same
    card always lands the same side of the random cut. Requires a C-slot
    symbol — without one there's nothing to watermark with."""
    if not has_c_symbol:
        return False
    h = hashlib.sha256(f"watermark:{card_id}".encode()).digest()
    val = int.from_bytes(h[:8], "big") / float(1 << 64)
    return val < rate


def symbol_composite_args(sym: str, x: int, y: int, pointsize: int) -> list[str]:
    """Build the magick args to composite ONE symbol glyph onto the art.
    Bold weight via FONT_BOLD + larger pointsize, screen blend for art
    integration. No -stroke — stroke would leak into subsequent text
    annotations and break title/type uniformity across the corpus."""
    return [
        "(",
        "-background", "none", "-fill", "white",
        "-font", FONT_BOLD, "-pointsize", str(pointsize),
        f"label:{sym}",
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
                # JPG icons arrive with white backgrounds (no alpha). Convert
                # near-white to transparent before recolor so we silhouette
                # the actual shape, not the bounding rectangle. Harmless on
                # ICO/PNG icons that already carry alpha.
                "-fuzz", "8%", "-transparent", "white",
                # Recolor RGB only, leave alpha intact — without -channel
                # scoping, +level-colors flattens the alpha and we lose the
                # silhouette entirely (probed 2026-06-09).
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

    # Pre-render abilities to a temp file so we can measure its height and
    # size the bottom banner to fit. -trim removes the transparent margin
    # caption: leaves around the text.
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
        # Top banner — solid themed color
        "(", "-size", f"{WIDTH}x{TOP_BANNER_H}", f"xc:{bg_rgba}", ")",
        "-gravity", "north", "-composite",
        # Bottom banner — dynamic height, fits content
        "(", "-size", f"{WIDTH}x{bottom_banner_h}", f"xc:{bg_rgba}", ")",
        "-gravity", "south", "-composite",
    ]

    # Watermark variant: large faded C-slot symbol behind the rules text.
    # Composited BEFORE the text so text sits on top of it.
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

    # Symbol column: stack symbols down the left edge, bold + screen blend.
    # No stroke — that would leak into subsequent text annotations.
    elif symbols:
        y = SYMBOL_TOP_PAD
        for sym in symbols:
            cmd += symbol_composite_args(sym, SYMBOL_LEFT_PAD, y, SYMBOL_POINTSIZE)
            y += SYMBOL_POINTSIZE + SYMBOL_GAP

    # Name top-left, left-aligned. NOT bold — FONT_REG so title looks the
    # same across every card in the corpus (symbol-bearing or not).
    name_x = (SYMBOL_LEFT_PAD + SYMBOL_POINTSIZE + 6) if (symbols and not use_watermark) else 10
    cmd += [
        "-gravity", "northwest", "-font", FONT_REG,
        "-pointsize", str(NAME_POINTSIZE), "-fill", fg,
        "-annotate", f"+{name_x}+12", name,
    ]

    # Cost top-right. Cost text is NOT bold — uniform with title and type.
    if cost_raw:
        tokens = cost_tokens(cost_raw)
        strip_path = f"/tmp/cost-strip-{os.getpid()}.png"
        if render_cost_strip(tokens, FONT_REG, COST_POINTSIZE, fg,
                             COST_POINTSIZE + 4, strip_path):
            cmd += [
                "(", strip_path, ")",
                "-gravity", "northeast", "-geometry", "+10+14", "-composite",
            ]

    # Type line: bottom-left aligned, NOT bold (FONT_REG, uniform with title).
    if type_line:
        BOT_PAD = 10
        GAP = 8 if abilities_h > 0 else 0
        type_y = BOT_PAD + abilities_h + GAP + 4  # +4 = descender clearance
        cmd += [
            "-gravity", "southwest", "-font", FONT_REG,
            "-pointsize", str(TYPE_POINTSIZE), "-fill", fg,
            "-annotate", f"+10+{type_y}", type_line,
        ]

    # Abilities image composited at bottom-left of card.
    if abilities_strip:
        cmd += [
            "(", abilities_strip, ")",
            "-gravity", "southwest", "-geometry", "+10+10", "-composite",
        ]

    # Stats badge bottom-right — NOT bold, no stroke. Uniform with title/type.
    if stats:
        cmd += [
            "-gravity", "southeast", "-font", FONT_REG,
            "-pointsize", str(STATS_POINTSIZE), "-fill", fg,
            "-annotate", "+8+6", stats,
        ]

    cmd += [png_path]
    subprocess.run(cmd, check=True)


# ---------------------------------------------------------------------
# Color tint via ImageMagick — second layer of color enforcement.
# ---------------------------------------------------------------------

def tint_rgb(colors: list[str]) -> tuple[int, int, int] | None:
    """Compute target RGB for the post-process tint. None = no tint (colorless)."""
    known = [c for c in colors if c in COLOR_RGB]
    if not known:
        return None
    rs, gs, bs = zip(*(COLOR_RGB[c] for c in known))
    return (sum(rs) // len(rs), sum(gs) // len(gs), sum(bs) // len(bs))


def tint_to_card_color(png_path: str, colors: list[str], strength: int) -> None:
    """Pull the generated image toward the card's color average.

    Two-pass: -colorize flat-blends with the target color (locks hue strongly),
    then -modulate re-saturates so the result stays loud rather than flat.
    strength is the colorize percentage (0-100). 35-50 keeps painterly chaos
    visible while clearly biasing the dominant hue. 0 skips.
    """
    if strength <= 0:
        return
    rgb = tint_rgb(colors)
    if rgb is None:
        return
    r, g, b = rgb
    subprocess.run([
        "magick", png_path,
        # 1. flat colorize blend toward target — uniform across luminances,
        #    stronger color-lock than -tint (which only biases midtones).
        "-fill", f"rgb({r},{g},{b})",
        "-colorize", str(strength),
        # 2. re-saturate so the blended image isn't flat — keep the chaos.
        "-modulate", "100,140,100",
        png_path,
    ], check=True)


# ---------------------------------------------------------------------
# Card frame — last post-process step. Near-black thin border + slight
# corner rounding, applied uniformly across every card in the corpus.
# Visual unity: every TSOT card has the same frame; the gen_ai owns
# the interior, the frame is ours.
# ---------------------------------------------------------------------

FRAME_RADIUS = 14           # corner rounding radius — sized to keep curve
                            # visible past the 4px border (14-4=10px of curve)
FRAME_BORDER = 4            # visible border thickness (doubled from 2)
FRAME_COLOR = "#0a0a0a"     # near-black

def apply_card_frame(png_path: str) -> None:
    w, h = WIDTH, HEIGHT
    r = FRAME_RADIUS
    # Stroke width is 2× the visible thickness because the outer half lands
    # in the rounded-corner cut region (alpha=0 after DstIn) and disappears.
    stroke_w = FRAME_BORDER * 2
    subprocess.run([
        "magick", png_path,
        # 1. Stroke the border on top of the image.
        "-fill", "none",
        "-stroke", FRAME_COLOR,
        "-strokewidth", str(stroke_w),
        "-draw", f"roundRectangle 0,0 {w - 1},{h - 1} {r},{r}",
        # 2. Cut the corners with a rounded-rect alpha mask (DstIn keeps
        #    the intersection of source alpha and mask alpha).
        "(",
        "-size", f"{w}x{h}", "xc:none",
        "-fill", "white",
        "-draw", f"roundRectangle 0,0 {w - 1},{h - 1} {r},{r}",
        ")",
        "-compose", "DstIn", "-composite",
        png_path,
    ], check=True)


# ---------------------------------------------------------------------
# Hole carving via ImageMagick.
# ---------------------------------------------------------------------

def carve_holes(png_path: str, holes: list[str]) -> None:
    args = ["magick", png_path, "-alpha", "set"]
    for slot in holes:
        x, y, w, h = slot_rect(slot)
        args += ["-region", f"{w}x{h}+{x}+{y}",
                 "-channel", "A", "-evaluate", "set", "0"]
    args += ["+channel", png_path]
    subprocess.run(args, check=True)


# ---------------------------------------------------------------------
# Entry point.
# ---------------------------------------------------------------------

def main() -> int:
    sd_bin = os.environ.get("SD_BIN") or DEFAULT_SD_BIN
    sd_model = os.environ.get("SD_MODEL") or DEFAULT_SD_MODEL
    if not Path(sd_bin).exists():
        sys.stderr.write(f"error: sd-cli not found at {sd_bin} — set SD_BIN or install it there\n")
        return 1
    if not Path(sd_model).exists():
        sys.stderr.write(f"error: model not found at {sd_model} — set SD_MODEL or download it there\n")
        return 1
    sd_lora = os.environ.get("SD_LORA", "lcm-lora-sdv1-5")
    sd_lora_dir = os.environ.get("SD_LORA_DIR", str(Path(sd_model).parent))

    Path(ART_DIR).mkdir(exist_ok=True)
    cards = load_cards(CARDS_DIR)
    pick_seed = int.from_bytes(os.urandom(4), "big")
    target = pick_target(cards, ART_DIR, seed=pick_seed)
    if target is None:
        print("all cards have art")
        return 0

    prompt, negative = build_prompt(target)
    seed = card_seed(target["id"])
    out = f"{ART_DIR}/{target['id']}_{WIDTH}_{HEIGHT}.png"

    # Append LCM LoRA so the 4-step / cfg-scale=1 sampling actually converges
    # on SD 1.5 base. Without it the output is noise.
    prompt_with_lora = f"{prompt} <lora:{sd_lora}:1>"

    bleed_pct = int(os.environ.get("SD_BLEED", "15"))
    gen_w, gen_h, crop_x, crop_y = bleed_dimensions(WIDTH, HEIGHT, bleed_pct)

    print(f"→ {target['id']}  (seed={seed})")
    print(f"  prompt: {prompt_with_lora}")
    if bleed_pct > 0:
        print(f"  bleed {bleed_pct}%: generating {gen_w}x{gen_h}, cropping to {WIDTH}x{HEIGHT}")

    sd_args = [
        sd_bin, "-m", sd_model,
        "-p", prompt_with_lora, "-n", negative,
        "-W", str(gen_w), "-H", str(gen_h),
        "--steps", "4", "--sampling-method", "lcm", "--cfg-scale", "1",
        "--lora-model-dir", sd_lora_dir,
        # Metal backend on Apple Silicon lacks the ADD op needed by the
        # default "apply lora immediately" path → SIGABRT in apply_loras.
        # at_runtime routes through a different op graph that Metal supports.
        "--lora-apply-mode", "at_runtime",
        "-s", str(seed), "-o", out,
    ]

    # Throttle by default so the machine stays responsive during generation.
    # SD_FAST=1 to override (faster but unusable).
    if os.environ.get("SD_FAST", "0") == "0":
        threads = os.environ.get("SD_THREADS", "2")
        vram_reserve = os.environ.get("SD_VRAM_RESERVE", "3.0")
        sd_args += [
            "-t", str(threads),
            # Negative max-vram = auto-detect and KEEP this many GiB free.
            "--max-vram", f"-{vram_reserve}",
            # Push CLIP + VAE to CPU so they don't compete with the display
            # compositor for the GPU. Slower per-step but the desktop stays
            # responsive. On Apple Silicon unified memory this only helps
            # compute scheduling, not total memory pressure.
            "--clip-on-cpu",
            "--vae-on-cpu",
            # NOTE: --mmap segfaults the Metal LoRA at_runtime path on
            # safetensors weights (probed 2026-06-09). Don't add it back.
        ]
        print(f"  throttled: -t {threads}, --max-vram -{vram_reserve}, CLIP+VAE on CPU (SD_FAST=1 to disable)")

    # TAESD — tiny autoencoder for fast VAE decode (~8s → ~1s). Auto-enabled
    # when the file is present. SD_TAESD overrides the path.
    sd_taesd = os.environ.get("SD_TAESD") or DEFAULT_SD_TAESD
    if Path(sd_taesd).exists():
        sd_args += ["--taesd", sd_taesd]
        print(f"  taesd: {sd_taesd}")

    subprocess.run(sd_args, check=True)

    if bleed_pct > 0:
        # Crop SD's self-drawn borders off, leaving the center as full-bleed art.
        subprocess.run([
            "magick", out, "-crop",
            f"{WIDTH}x{HEIGHT}+{crop_x}+{crop_y}", "+repage", out,
        ], check=True)

    tint_strength = int(os.environ.get("SD_TINT", "25"))
    if tint_strength > 0 and target.get("colors"):
        tint_to_card_color(out, target["colors"], tint_strength)
        print(f"  tinted toward {','.join(target['colors'])} @ {tint_strength}%")

    if os.environ.get("SD_TEXT", "1") != "0":
        overlay_text(out, target)
        print(f"  overlaid text")

    holes = target.get("holes") or []
    if holes:
        carve_holes(out, holes)
        print(f"  carved holes: {','.join(holes)}")

    # Card frame is the LAST step so it sits on top of everything (banners,
    # holes, art) and the rounded corners cut every pixel uniformly.
    apply_card_frame(out)

    print(f"✓ {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
