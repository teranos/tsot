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

import datetime
import hashlib
import json
import os
import random
import subprocess
import sys
from pathlib import Path

# All rendering-layer functions and constants (WIDTH, HEIGHT, slot grid,
# themes, format_*, overlay_text, apply_card_frame, carve_holes, tint, etc.)
# live in card_render — see tools/card_render.py.
from card_render import *  # noqa: F401, F403


CARDS_DIR = "cards"
ART_DIR = "gen_art"

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

def card_pngs(art_dir: str, card_id: str) -> list[Path]:
    """All PNG files for a card in art_dir, sorted by name."""
    return sorted(Path(art_dir).glob(f"{card_id}_*.png"))


def latest_png(art_dir: str, card_id: str) -> Path | None:
    pngs = card_pngs(art_dir, card_id)
    if not pngs:
        return None
    return max(pngs, key=lambda p: p.stat().st_mtime)


def next_output_path(art_dir: str, card_id: str, w: int, h: int) -> str:
    """Next available output path. First gen: {id}_{W}_{H}.png. Subsequent:
    {id}_{W}_{H}_{N}.png where N is the smallest integer ≥ 2 that doesn't
    collide. Existing PNGs are never overwritten."""
    base = Path(art_dir) / f"{card_id}_{w}_{h}.png"
    if not base.exists():
        return str(base)
    n = 2
    while True:
        candidate = Path(art_dir) / f"{card_id}_{w}_{h}_{n}.png"
        if not candidate.exists():
            return str(candidate)
        n += 1


def card_lua_path(card_id: str) -> Path:
    return Path(CARDS_DIR) / f"{card_id}.lua"


def read_png_property(png_path: Path, key: str) -> str | None:
    """Read a single PNG property via magick identify. Returns None if the
    property is absent, the file isn't a real PNG, or magick errors."""
    try:
        result = subprocess.check_output(
            ["magick", "identify", "-format", f"%[property:{key}]", str(png_path)],
            text=True, stderr=subprocess.DEVNULL,
        ).strip()
        return result if result else None
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None


def is_stale(card_id: str, art_dir: str) -> bool:
    """The card's .lua has been edited more recently than its latest PNG —
    the image is out of date with the card definition."""
    png = latest_png(art_dir, card_id)
    if png is None:
        return False
    lua = card_lua_path(card_id)
    if not lua.exists():
        return False
    return lua.stat().st_mtime > png.stat().st_mtime


def pick_target(cards: list[dict], art_dir: str, seed: int) -> dict | None:
    """Priority order:
    1. STALE: card .lua mtime > latest PNG mtime (image out of date).
    2. FRESH: no PNG exists for this card yet.
    3. NO_METADATA: latest PNG lacks tsot.* properties (pre-metadata commit).
    4. OLDEST: latest PNG has the oldest tsot.timestamp.
    Step 1 always wins, even over cards with no art yet."""
    rng = random.Random(seed)

    # Tier 1: stale (top priority — always)
    stale = [c for c in cards
             if c.get("frame") != "transparent"
             and is_stale(c["id"], art_dir)]
    if stale:
        return rng.choice(sorted(stale, key=lambda c: c["id"]))

    # Tier 2: no PNG yet
    fresh = [c for c in cards
             if c.get("frame") != "transparent"
             and not card_pngs(art_dir, c["id"])]
    if fresh:
        return rng.choice(sorted(fresh, key=lambda c: c["id"]))

    # Tier 3 / 4: corpus is complete. Walk the metadata.
    no_meta: list[dict] = []
    has_meta: list[tuple[str, dict]] = []
    for c in cards:
        if c.get("frame") == "transparent":
            continue
        png = latest_png(art_dir, c["id"])
        if png is None:
            continue
        ts = read_png_property(png, "tsot.timestamp")
        if ts is None:
            no_meta.append(c)
        else:
            has_meta.append((ts, c))

    if no_meta:
        return rng.choice(sorted(no_meta, key=lambda c: c["id"]))

    if has_meta:
        has_meta.sort(key=lambda x: x[0])  # oldest first
        return has_meta[0][1]

    return None


def card_seed(card_id: str) -> int:
    return int(hashlib.sha256(card_id.encode()).hexdigest()[:8], 16)


# ---------------------------------------------------------------------
# PNG metadata — every generation tags the output with full provenance
# so a card image is self-describing: card identity, generation params,
# prompt text, pipeline commit, timestamp. Stored as PNG tEXt chunks.
# ---------------------------------------------------------------------

def git_short_hash() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            text=True, stderr=subprocess.DEVNULL,
        ).strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def build_metadata(card: dict, seed: int, prompt: str, negative: str,
                    sd_model: str, sd_lora: str, bleed_pct: int,
                    tint_strength: int, watermark_active: bool,
                    theme_bg: tuple[int, int, int]) -> dict[str, str]:
    """Full provenance dict to embed as PNG tEXt chunks. Every value is
    a string — magick -set property: needs strings."""
    syms_raw = card.get("symbols")
    if syms_raw is None and card.get("symbol"):
        syms_raw = [card["symbol"]]
    symbols_str = json.dumps(syms_raw or [], ensure_ascii=False)
    return {
        "tsot.card.id":              str(card.get("id", "")),
        "tsot.card.name":            str(card.get("name", "")),
        "tsot.card.type":            str(card.get("type", "")),
        "tsot.card.subtypes":        ",".join(card.get("subtypes") or []),
        "tsot.card.colors":          ",".join(card.get("colors") or []),
        "tsot.card.symbols":         symbols_str,
        "tsot.card.holes":           ",".join(card.get("holes") or []),
        "tsot.gen.seed":             str(seed),
        "tsot.gen.prompt":           prompt,
        "tsot.gen.negative":         negative,
        "tsot.gen.sd_model":         Path(sd_model).name,
        "tsot.gen.sd_lora":          sd_lora,
        "tsot.gen.steps":            "4",
        "tsot.gen.sampler":          "lcm",
        "tsot.gen.cfg_scale":        "1",
        "tsot.gen.lora_apply_mode":  "at_runtime",
        "tsot.post.bleed_pct":       str(bleed_pct),
        "tsot.post.tint_strength":   str(tint_strength),
        "tsot.post.watermark_active": str(watermark_active),
        "tsot.post.theme_bg":        ",".join(str(c) for c in theme_bg),
        "tsot.pipeline.git_commit":  git_short_hash(),
        "tsot.timestamp":            datetime.datetime.now(datetime.timezone.utc)
                                         .strftime("%Y-%m-%dT%H:%M:%SZ"),
    }


def apply_metadata(png_path: str, metadata: dict[str, str]) -> None:
    """Write metadata properties into the PNG via magick -set property:."""
    cmd = ["magick", png_path]
    for key, value in metadata.items():
        cmd += ["-set", f"property:{key}", value]
    cmd += [png_path]
    subprocess.run(cmd, check=True)


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
    out = next_output_path(ART_DIR, target["id"], WIDTH, HEIGHT)

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

    # Embed full provenance into the PNG metadata.
    theme = text_theme(target.get("colors") or [])
    watermark_active = card_uses_watermark_variant(
        target.get("id", ""),
        c_slot_symbol(target) is not None,
    )
    metadata = build_metadata(
        card=target, seed=seed, prompt=prompt, negative=negative,
        sd_model=sd_model, sd_lora=sd_lora,
        bleed_pct=bleed_pct, tint_strength=tint_strength,
        watermark_active=watermark_active,
        theme_bg=theme["bg"],
    )
    apply_metadata(out, metadata)

    print(f"✓ {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
