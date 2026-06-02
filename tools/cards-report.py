#!/usr/bin/env python3
"""tsot pool + archetypes dashboard.

Two reports, one HTML output (`card-pool.html`):

  1. Card pool — reads every cards/*.lua, aggregates by color / cost /
     type / subtype. Surfaces design gaps ("where are the gaps in red",
     "is purple over-represented at 1-cost", "which subtypes have one
     card and are basically dead").

  2. Deck archetypes — loads every EvolvedDeck JSON in baselines/ and
     champions/, clusters them by Jaccard similarity on card-id sets,
     surfaces cluster signatures + a full Jaccard heatmap. Designed to
     make `curate-baselines` output meaningful: which clusters did the
     unmatched champions form, and at what threshold would they have
     matched?

Card files contain Lua function bodies (event handlers, static blocks);
we shell out to lua5.4 once with a small driver that dofiles each card
and emits JSON-Lines with only the data fields. Deck JSONs load with
the stdlib `json` module. Everything else runs in Python.

Usage:
    python3 tools/cards-report.py [--dir cards] [--out card-pool.html]
                                  [--baselines DIR] [--champions DIR]
                                  [--threshold 0.5] [--no-archetypes]
"""
from __future__ import annotations

import argparse
import html
import json
import math
import os
import subprocess
import sys
from pathlib import Path

# ---------------------------------------------------------------------
# Card loading via lua5.4 subprocess
# ---------------------------------------------------------------------
#
# Cards are .lua files that return a table containing data and function
# fields. We need Lua to evaluate them, but only the data is interesting
# to the dashboard. The driver below dofiles each card and prints one
# JSON object per line; functions and unrepresentable values are
# dropped.

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
    result._path = path
    io.write(encode(result), "\n")
  end
end
"""


def load_cards(directory: str) -> tuple[list[dict], list[dict]]:
    # `lua5.4 - <dir>` reads the script from stdin so the directory can
    # be passed as arg[1] without lua mistaking it for a script file.
    proc = subprocess.run(
        ["lua5.4", "-", directory],
        input=LUA_DRIVER,
        capture_output=True,
        text=True,
        check=False,
    )
    cards: list[dict] = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line:
            cards.append(json.loads(line))
    errors: list[dict] = []
    for line in proc.stderr.splitlines():
        if line.startswith("ERR\t"):
            parts = line.split("\t", 2)
            if len(parts) == 3:
                errors.append({"path": parts[1], "err": parts[2]})
    return cards, errors


# ---------------------------------------------------------------------
# Curve data
# ---------------------------------------------------------------------

CURVE_MAX_TURN = 14


def load_curve(path: str = "card-curve.json") -> dict | None:
    if not os.path.exists(path):
        return None
    with open(path) as f:
        data = json.load(f)
    out = {
        "n_games": data.get("n_games", 0),
        "seed": data.get("seed", "?"),
        "card_curves": {},
    }
    for cid, info in (data.get("card_curves") or {}).items():
        turns = {int(t): int(c) for t, c in (info.get("turns") or {}).items()}
        out["card_curves"][cid] = {"plays": int(info.get("plays", 0)), "turns": turns}
    return out


def curve_for(curve: dict | None, card_id: str) -> dict | None:
    if not curve:
        return None
    return curve["card_curves"].get(card_id)


def curve_median_turn(c: dict | None) -> float | None:
    if not c or not c.get("turns"):
        return None
    flat = []
    for t, count in c["turns"].items():
        flat.extend([t] * count)
    if not flat:
        return None
    flat.sort()
    m = len(flat) // 2
    if len(flat) % 2 == 0:
        return (flat[m - 1] + flat[m]) / 2
    return float(flat[m])


def curve_mean_turn(c: dict | None) -> float | None:
    if not c or not c.get("turns"):
        return None
    total = sum(t * count for t, count in c["turns"].items())
    n = sum(c["turns"].values())
    return total / n if n else None


BLOCKS = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"]


def curve_histogram(c: dict | None) -> str:
    if not c or not c.get("turns"):
        return ""
    max_count = max(c["turns"].values(), default=0)
    if max_count == 0:
        return ""
    out = []
    for t in range(1, CURVE_MAX_TURN + 1):
        n = c["turns"].get(t, 0)
        if n == 0:
            out.append("·")
        else:
            idx = max(1, math.ceil((n / max_count) * len(BLOCKS)))
            out.append(BLOCKS[idx - 1])
    return "".join(out)


# ---------------------------------------------------------------------
# Deck loading + Jaccard clustering (archetypes section)
# ---------------------------------------------------------------------
#
# Decks come from `baselines/*.json` and `champions/*.json` — the
# EvolvedDeck shape produced by `evolve` / `curate-baselines`. We only
# need `label`, `fitness`, `card_ids`; everything else is ignored.


def load_decks(baselines_dir: str, champions_dir: str) -> list[dict]:
    out: list[dict] = []
    for source, dir_ in (("baseline", baselines_dir), ("champion", champions_dir)):
        path = Path(dir_)
        if not path.is_dir():
            continue
        for fp in sorted(path.glob("*.json")):
            try:
                data = json.loads(fp.read_text())
            except (OSError, json.JSONDecodeError):
                continue
            ids = list(data.get("card_ids") or [])
            out.append({
                "name": fp.stem,
                "source": source,
                "label": data.get("label", "unknown"),
                "fitness": float(data.get("fitness") or 0),
                "ids_list": ids,
                "ids_set": frozenset(ids),
            })
    return out


def jaccard(a: frozenset, b: frozenset) -> float:
    if not a and not b:
        return 1.0
    inter = len(a & b)
    union = len(a | b)
    return inter / union if union else 1.0


class UnionFind:
    def __init__(self, n: int):
        self.parent = list(range(n))

    def find(self, i: int) -> int:
        while self.parent[i] != i:
            self.parent[i] = self.parent[self.parent[i]]
            i = self.parent[i]
        return i

    def union(self, i: int, j: int) -> None:
        ri, rj = self.find(i), self.find(j)
        if ri != rj:
            self.parent[ri] = rj


def cluster_decks(decks: list[dict], threshold: float) -> tuple[list[list[float]], list[list[int]]]:
    """Returns (jaccard matrix, list of clusters sorted by size desc)."""
    n = len(decks)
    jmat = [[1.0] * n for _ in range(n)]
    for i in range(n):
        for j in range(i + 1, n):
            v = jaccard(decks[i]["ids_set"], decks[j]["ids_set"])
            jmat[i][j] = v
            jmat[j][i] = v

    uf = UnionFind(n)
    for i in range(n):
        for j in range(i + 1, n):
            if jmat[i][j] >= threshold:
                uf.union(i, j)

    buckets: dict[int, list[int]] = {}
    for i in range(n):
        buckets.setdefault(uf.find(i), []).append(i)

    clusters = sorted(
        buckets.values(),
        key=lambda c: (-len(c), decks[c[0]]["name"]),
    )
    return jmat, clusters


def cluster_centroid(idxs: list[int], jmat: list[list[float]]) -> tuple[int, float]:
    """Member with highest average Jaccard to the rest of the cluster."""
    if len(idxs) == 1:
        return idxs[0], 0.0
    best_i, best_avg = idxs[0], -1.0
    for i in idxs:
        others = [j for j in idxs if j != i]
        avg = sum(jmat[i][j] for j in others) / len(others) if others else 0.0
        if avg > best_avg:
            best_avg = avg
            best_i = i
    return best_i, best_avg


def cluster_signature(idxs: list[int], decks: list[dict]) -> list[tuple[str, int]]:
    """Cards appearing in ≥ ceil(|cluster| / 2) of the cluster's decks."""
    counts: dict[str, int] = {}
    for i in idxs:
        for cid in decks[i]["ids_set"]:
            counts[cid] = counts.get(cid, 0) + 1
    half = math.ceil(len(idxs) / 2)
    sig = [(cid, c) for cid, c in counts.items() if c >= half]
    sig.sort(key=lambda x: (-x[1], x[0]))
    return sig


def nearest_other(idx: int, jmat: list[list[float]]) -> tuple[int | None, float]:
    n = len(jmat)
    best_j, best_v = None, -1.0
    for j in range(n):
        if j != idx and jmat[idx][j] > best_v:
            best_v = jmat[idx][j]
            best_j = j
    return best_j, best_v


def jaccard_cell_style(t: float) -> str:
    """Red below 0.5, green at 0.5+, brighter green nearer 1.0."""
    t = max(0.0, min(1.0, t))
    if t < 0.5:
        k = t * 2
        r = int(80 + (1 - k) * 60)
        g = int(60 + k * 40)
        b = 60
    else:
        k = (t - 0.5) * 2
        r = int(40 + (1 - k) * 40)
        g = int(120 + k * 50)
        b = 40
    return f"background: rgb({r},{g},{b}); color: #eee;"


# ---------------------------------------------------------------------
# Card helpers
# ---------------------------------------------------------------------


def card_total_cost(card: dict):
    cost = card.get("cost") or []
    if not cost:
        return 0
    total = 0
    has_x = False
    for c in cost:
        if c.get("is_x"):
            has_x = True
        elif "amount" in c:
            total += c["amount"]
    return "X" if has_x else total


def format_cost(card: dict) -> str:
    cost = card.get("cost") or []
    if not cost:
        return "—"
    parts = []
    for c in cost:
        amt = "X" if c.get("is_x") else str(c.get("amount", 0))
        parts.append(f"{amt} {c.get('source', '?')}")
    return " + ".join(parts)


def card_colors(card: dict) -> list[str]:
    return card.get("colors") or []


def card_subtypes(card: dict) -> list[str]:
    return card.get("subtypes") or []


def card_type(card: dict) -> str:
    # "spell" is a legacy Lua-source alias for "sorcery". The Rust parser
    # maps both to (CardType::Spell, Timing::Sorcery); fold here so the
    # dashboard doesn't expose a distinction that doesn't exist at runtime.
    t = card.get("type")
    if t == "spell":
        return "sorcery"
    return t or "?"


def has_keyword(card: dict, kw: str) -> bool:
    for a in card.get("abilities") or []:
        if isinstance(a, str) and kw in a.lower():
            return True
    return False


KNOWN_KEYWORDS = [
    "flying", "reach", "haste", "vigilance",
    "unblockable", "defender", "cannot-block", "first strike",
]

# Order matters: rendered top-to-bottom in matrices. Current shipping
# colors first, then planned-but-unshipped (will show as empty rows,
# which is the point — surfaces the design TODO).
KNOWN_COLORS = [
    "red", "blue", "green", "purple", "black", "white",
    "pink", "orange", "azure",
]
# `transparent` is a frame attribute, not a color (see RULES C.13). Cards
# declare it via `frame = "transparent"`. Rendered as a chip in Pool
# summary and as a swatch in the All-cards table, parallel to colors but
# separate from color-identity aggregates.
KNOWN_FRAMES = ["transparent"]
# `face` is a card-surface treatment. shiny/holo are purely cosmetic;
# `glow` carries V.9 (visibility-through-stacks) semantics but lives on
# the same field. Migration of existing glow cards from `colors` to
# `face` is a follow-up slice — for now glow appears in this list so
# the dashboard already knows where to render it when migration lands.
KNOWN_FACES = ["shiny", "holo", "glow"]
KNOWN_TYPES = ["creature", "instant", "sorcery", "artifact", "mutation"]
KNOWN_SOURCES = ["hand", "mill", "graveyard", "sacrifice", "self"]
COST_BUCKETS: list = [0, 1, 2, 3, 4, 5, "6+", "X"]


def bucket_cost(tc):
    if tc == "X":
        return "X"
    if isinstance(tc, (int, float)):
        if tc >= 6:
            return "6+"
        return int(tc)
    return 0


# ---------------------------------------------------------------------
# Aggregates
# ---------------------------------------------------------------------


def build_aggregates(cards: list[dict]) -> dict:
    all_colors = list(KNOWN_COLORS) + ["colorless"]
    agg = {
        "total": len(cards),
        "by_color": {c: 0 for c in all_colors},
        "by_frame": {f: 0 for f in KNOWN_FRAMES},
        "by_face": {f: 0 for f in KNOWN_FACES},
        "by_type": {},
        "by_subtype": {},
        "by_keyword": {kw: 0 for kw in KNOWN_KEYWORDS},
        "color_x_cost": {c: {b: 0 for b in COST_BUCKETS} for c in all_colors},
        "color_x_type": {c: {t: 0 for t in KNOWN_TYPES} for c in all_colors},
        "source_mix": {c: {"total": 0, **{s: 0 for s in KNOWN_SOURCES}} for c in all_colors},
        "multicolor": {"single": 0, "hybrid": 0, "colorless": 0},
    }

    for card in cards:
        t = card_type(card)
        agg["by_type"][t] = agg["by_type"].get(t, 0) + 1

        f = card.get("frame")
        if f:
            agg["by_frame"][f] = agg["by_frame"].get(f, 0) + 1

        for fa in card.get("face") or []:
            agg["by_face"][fa] = agg["by_face"].get(fa, 0) + 1

        cs = card_colors(card)
        if len(cs) == 0:
            agg["by_color"]["colorless"] += 1
            agg["multicolor"]["colorless"] += 1
        elif len(cs) == 1:
            agg["multicolor"]["single"] += 1
        else:
            agg["multicolor"]["hybrid"] += 1
        for color in cs:
            agg["by_color"][color] = agg["by_color"].get(color, 0) + 1

        tc = card_total_cost(card)
        bucket = bucket_cost(tc)
        cost_targets = ["colorless"] if not cs else cs
        for color in cost_targets:
            cx = agg["color_x_cost"].setdefault(color, {b: 0 for b in COST_BUCKETS})
            cx[bucket] = cx.get(bucket, 0) + 1
            ct = agg["color_x_type"].setdefault(color, {t: 0 for t in KNOWN_TYPES})
            ct[t] = ct.get(t, 0) + 1

        if card.get("cost"):
            for comp in card["cost"]:
                src = comp.get("source", "?")
                amt = 1 if comp.get("is_x") else comp.get("amount", 0)
                for color in cost_targets:
                    sm = agg["source_mix"].setdefault(
                        color, {"total": 0, **{s: 0 for s in KNOWN_SOURCES}}
                    )
                    sm[src] = sm.get(src, 0) + amt
                    sm["total"] += amt

        for st in card_subtypes(card):
            key = st.lower()
            entry = agg["by_subtype"].setdefault(key, {"cards": [], "colors": set()})
            entry["cards"].append(card)
            for color in cs:
                entry["colors"].add(color)

        for kw in KNOWN_KEYWORDS:
            if has_keyword(card, kw):
                agg["by_keyword"][kw] += 1

    return agg


# ---------------------------------------------------------------------
# HTML rendering
# ---------------------------------------------------------------------


def esc(s) -> str:
    # Match the Lua source's gsub output exactly: `'` → `&#39;`, not Python's
    # default `&#x27;`. Keeps diffs against the Lua reference output clean.
    return html.escape("" if s is None else str(s), quote=True).replace("&#x27;", "&#39;")


def color_cell_style(value: int, row_max: int) -> str:
    if row_max == 0:
        return ""
    t = value / row_max
    r = int((1 - t) * 28 + 24)
    g = int(t * 160 + 30)
    b = int((1 - t) * 28 + 24)
    return f"background: rgb({r},{g},{b}); color: #eee;"


SWATCHES = {
    "red": "#d4604e", "blue": "#5d8ec4", "green": "#6fa86a",
    "purple": "#9a6bbd", "black": "#3a3a3a", "white": "#d6d4c8",
    "colorless": "#86878a",
    "pink": "#d97ea8", "orange": "#d9885a", "azure": "#5ec4d4",
    "glow": "#c8e88a",
}


def color_swatch(color: str) -> str:
    if color == "glow":
        style = f"background:{SWATCHES['glow']};box-shadow:0 0 4px {SWATCHES['glow']};"
    else:
        hex_ = SWATCHES.get(color, "#888")
        style = f"background:{hex_};"
    return (
        f'<span style="display:inline-block;width:10px;height:10px;{style}'
        f'border-radius:2px;margin-right:4px;vertical-align:middle;"></span>{esc(color)}'
    )


def frame_swatch(frame: str) -> str:
    """Visual badge for a frame attribute (transparent: checkerboard)."""
    if frame == "transparent":
        style = "background:repeating-conic-gradient(#444 0% 25%, #222 0% 50%) 50% / 6px 6px;"
    else:
        style = "background:#888;"
    return (
        f'<span style="display:inline-block;width:10px;height:10px;{style}'
        f'border-radius:2px;margin-right:4px;vertical-align:middle;"></span>{esc(frame)}'
    )


def face_badge(face: str) -> str:
    """Visual badge for a face attribute. shiny/holo get gradient sparkle;
    glow gets the existing glow swatch style."""
    if face == "shiny":
        style = "background:linear-gradient(135deg,#e8e8e8,#9b9b9b,#fafafa);"
    elif face == "holo":
        style = "background:linear-gradient(135deg,#ff6ec7,#ffd84d,#6effd8,#6ec7ff);"
    elif face == "glow":
        style = f"background:{SWATCHES['glow']};box-shadow:0 0 4px {SWATCHES['glow']};"
    else:
        style = "background:#888;"
    return (
        f'<span style="display:inline-block;width:10px;height:10px;{style}'
        f'border-radius:50%;margin-right:4px;vertical-align:middle;"></span>{esc(face)}'
    )


def color_swatch_only(color: str) -> str:
    """Swatch box without the color name — for compact deck-identity cells."""
    if color == "transparent":
        style = "background:repeating-conic-gradient(#444 0% 25%, #222 0% 50%) 50% / 6px 6px;"
    elif color == "glow":
        style = f"background:{SWATCHES['glow']};box-shadow:0 0 4px {SWATCHES['glow']};"
    else:
        hex_ = SWATCHES.get(color, "#888")
        style = f"background:{hex_};"
    return (
        f'<span title="{esc(color)}" style="display:inline-block;width:9px;height:9px;{style}'
        f'border-radius:2px;vertical-align:middle;"></span>'
    )


def color_bar(counts: list[tuple[str, int]]) -> str:
    """Stacked horizontal bar: each color's width is proportional to its count.
    Hovering a segment shows `color: N`. Empty/zero deck → empty bar div."""
    total = sum(n for _, n in counts)
    if total == 0:
        return '<div class="cbar"></div>'
    segs = []
    for color, n in counts:
        if color == "transparent":
            bg = "repeating-conic-gradient(#444 0% 25%, #222 0% 50%) 50% / 6px 6px"
        elif color == "glow":
            bg = SWATCHES["glow"]
        else:
            bg = SWATCHES.get(color, "#888")
        pct = 100 * n / total
        segs.append(
            f'<span class="cbar-seg" data-label="{esc(color)}: {n}" '
            f'style="width:{pct:.3f}%;background:{bg};"></span>'
        )
    return f'<div class="cbar">{"".join(segs)}</div>'


def deck_color_counts(deck: dict, card_by_id: dict[str, dict]) -> list[tuple[str, int]]:
    """Color distribution for a deck. Multi-color cards count for each color;
    cards with no colors fall into "colorless". Returned in fixed KNOWN_COLORS
    order (+ colorless last, + any unknown colors trailing alphabetically) so
    bars line up across rows for visual comparison."""
    counts: dict[str, int] = {}
    for cid in deck["ids_list"]:
        card = card_by_id.get(cid)
        if not card:
            continue
        cs = card.get("colors") or []
        if not cs:
            counts["colorless"] = counts.get("colorless", 0) + 1
        else:
            for c in cs:
                counts[c] = counts.get(c, 0) + 1
    order = [*KNOWN_COLORS, "colorless"]
    out: list[tuple[str, int]] = []
    for c in order:
        if counts.get(c, 0) > 0:
            out.append((c, counts[c]))
    for c in sorted(counts):
        if c not in order:
            out.append((c, counts[c]))
    return out


def deck_symbol_counts(deck: dict, card_by_id: dict[str, dict]) -> list[tuple[str, int]]:
    """Symbol distribution. Cards without a symbol are dropped (not "no symbol")."""
    counts: dict[str, int] = {}
    for cid in deck["ids_list"]:
        card = card_by_id.get(cid)
        if not card:
            continue
        sym = card.get("symbol")
        if sym:
            counts[sym] = counts.get(sym, 0) + 1
    return sorted(counts.items(), key=lambda x: (-x[1], x[0]))


CSS = (Path(__file__).resolve().parent / "cards-report.css").read_text()


def card_tooltip(card: dict) -> str:
    name = card.get("name") or card.get("id") or ""
    meta_parts = []
    cs = card.get("colors") or []
    if cs:
        meta_parts.append(" ".join(cs))
    meta_parts.append(card_type(card))
    sts = card.get("subtypes") or []
    if sts:
        meta_parts.append("— " + "/".join(sts))
    meta = " ".join(meta_parts)

    out = [
        f'<span class="card-cell"><span class="card-id">{esc(card.get("id", ""))}</span>',
        '<span class="card-tooltip">',
        f'<div class="ct-name">{esc(name)}</div>',
    ]
    if meta:
        out.append(f'<div class="ct-meta">{esc(meta)}</div>')
    if card.get("cost"):
        out.append(f'<div class="ct-cost">cost: {esc(format_cost(card))}</div>')
    if card.get("stats"):
        s = card["stats"]
        out.append(f'<div class="ct-stats">stats: {s.get("x", 0)}/{s.get("y", 0)}</div>')
    if card.get("abilities"):
        out.append('<div class="ct-abilities">')
        for a in card["abilities"]:
            out.append(f'<div>{esc(a)}</div>')
        out.append('</div>')
    if card.get("flavor"):
        out.append(f'<div class="ct-flavor">{esc(card["flavor"])}</div>')
    out.append('</span></span>')
    return "".join(out)


def render_archetypes(
    parts: list[str],
    decks: list[dict],
    jmat: list[list[float]],
    clusters: list[list[int]],
    threshold: float,
    card_by_id: dict[str, dict],
) -> None:
    w = parts.append
    n = len(decks)
    n_baseline = sum(1 for d in decks if d["source"] == "baseline")
    n_champion = n - n_baseline

    w("<h2>Deck archetypes</h2>")
    w('<div class="meta">')
    w(f'<div>decks <b>{n}</b></div>')
    w(f'<div>baselines <b>{n_baseline}</b></div>')
    w(f'<div>champions <b>{n_champion}</b></div>')
    w(f'<div>threshold <b>{threshold:.2f}</b></div>')
    w(f'<div>clusters <b>{len(clusters)}</b></div>')
    w('</div>')

    # Cluster id lookup for the All-decks table below.
    cluster_id_of: dict[int, int] = {}
    for cid, idxs in enumerate(clusters, start=1):
        for i in idxs:
            cluster_id_of[i] = cid

    # Cluster roster
    w("<h3>Clusters</h3>")
    w('<p class="note">Greedy single-linkage at Jaccard ≥ ')
    w(f'{threshold:.2f}')
    w('. A "singleton" cluster (1 member) means that deck has no Jaccard-similar peer at this threshold — usually a distinct attractor or an outlier.</p>')

    for cid, idxs in enumerate(clusters, start=1):
        size = len(idxs)
        is_singleton = size == 1
        centroid_idx, centroid_avg = cluster_centroid(idxs, jmat)
        sig = cluster_signature(idxs, decks)

        cls = "cluster singleton" if is_singleton else "cluster"
        w(f'<div class="{cls}">')
        head = f'<h3>Cluster {cid} &middot; {size} deck{"" if size == 1 else "s"}'
        if not is_singleton:
            head += f' &middot; avg internal Jaccard {centroid_avg:.2f}'
        head += '</h3>'
        w(head)

        w('<div class="members">')
        w(f'representative: <span class="centroid">{esc(decks[centroid_idx]["name"])}'
          f' ({esc(decks[centroid_idx]["source"])})</span>')
        if not is_singleton:
            w(' &middot; <b>members:</b> ')
            w(", ".join(esc(decks[i]["name"]) for i in idxs))
        w('</div>')

        if sig:
            w('<div class="sigcards"><b>signature cards</b> (in ≥ ½ of cluster): ')
            shown = sig[:18]
            w(", ".join(f'{esc(cid_)} [{c}/{size}]' for cid_, c in shown))
            if len(sig) > 18:
                w(f' &middot; +{len(sig) - 18} more')
            w('</div>')

        if is_singleton:
            nj, nv = nearest_other(idxs[0], jmat)
            if nj is not None:
                w('<div class="sigcards">'
                  f'nearest other deck: <b>{esc(decks[nj]["name"])}</b> '
                  f'at Jaccard <b>{nv:.2f}</b>')
                if threshold * 0.7 <= nv < threshold:
                    w(f' &middot; <b>would join its cluster at threshold ≤ {nv:.2f}</b>')
                w('</div>')

        w('</div>')

    # Per-deck table
    w("<h3>All decks</h3>")
    w('<p class="note">Deck-level view. <em>colors</em> / <em>symbols</em> count cards by color identity and symbol — '
      'multi-color cards count once per color; cards without a symbol are dropped. '
      '<em>nearest</em> is the closest other deck by Jaccard.</p>')
    w('<table><thead><tr><th>deck</th><th>source</th><th>label</th>'
      '<th>colors</th><th>symbols</th>'
      '<th class="num">fitness</th>'
      '<th>nearest</th><th class="num">jaccard</th><th class="num">cluster</th></tr></thead><tbody>')
    for i in range(n):
        nj, nv = nearest_other(i, jmat)
        my_cluster_size = len(clusters[cluster_id_of[i] - 1])
        is_singleton = my_cluster_size == 1
        tr_cls = ' class="singleton"' if is_singleton else ''
        color_html = color_bar(deck_color_counts(decks[i], card_by_id))
        symbol_html = "".join(
            f'<span class="sym-chip"><span class="sym-glyph">{esc(s)}</span>'
            f'<span class="sym-cnt">{n_}</span></span>'
            for s, n_ in deck_symbol_counts(decks[i], card_by_id)
        )
        w(f'<tr{tr_cls}>')
        w(f'<td>{esc(decks[i]["name"])}</td>')
        w(f'<td class="source-{decks[i]["source"]}">{decks[i]["source"]}</td>')
        w(f'<td>{esc(decks[i]["label"])}</td>')
        w(f'<td class="ident">{color_html}</td>')
        w(f'<td class="ident">{symbol_html}</td>')
        w(f'<td class="num">{decks[i]["fitness"]:.3f}</td>')
        if nj is not None:
            w(f'<td>{esc(decks[nj]["name"])}</td>')
            w(f'<td class="num">{nv:.2f}</td>')
        else:
            w('<td></td><td class="num"></td>')
        w(f'<td class="num">#{cluster_id_of[i]}</td>')
        w('</tr>')
    w('</tbody></table>')

    # Heatmap
    w("<h3>Jaccard heatmap</h3>")
    w('<p class="note">Pairwise Jaccard similarity. Cells colored: red below 0.5, green at 0.5+, '
      'brighter green nearer 1.0.</p>')
    w('<table class="heatmap"><thead><tr><th></th>')
    for i in range(n):
        w(f'<th>{i + 1}</th>')
    w('</tr></thead><tbody>')
    for i in range(n):
        w(f'<tr><th>{i + 1} {esc(decks[i]["name"])}</th>')
        for j in range(n):
            v = jmat[i][j]
            if i == j:
                w(f'<td style="background:#3f4140;color:#868787">{v:.2f}</td>')
            else:
                w(f'<td style="{jaccard_cell_style(v)}">{v:.2f}</td>')
        w('</tr>')
    w('</tbody></table>')


def render_html(
    cards: list[dict],
    agg: dict,
    curve: dict | None,
    args: argparse.Namespace,
    load_errors: list[dict],
    decks: list[dict] | None = None,
    jmat: list[list[float]] | None = None,
    clusters: list[list[int]] | None = None,
) -> str:
    parts: list[str] = []
    w = parts.append

    w("<!DOCTYPE html><html lang=en><head><meta charset=utf-8>")
    w("<title>tsot — card pool</title>")
    w(f"<style>{CSS}</style>")
    w("</head><body>")
    w("<h1>tsot — card pool</h1>")
    w('<div class="meta">')
    w(f'<div>cards <b>{agg["total"]}</b></div>')
    w(f'<div>dir <b>{esc(args.dir)}</b></div>')
    if decks is not None:
        w(f'<div>decks <b>{len(decks)}</b></div>')
        w(f'<div>clusters <b>{len(clusters or [])}</b></div>')
    w('</div>')

    if load_errors:
        w('<div class="errors"><b>Load errors:</b><ul>')
        for e in load_errors:
            w(f'<li>{esc(e["path"])}: {esc(e["err"])}</li>')
        w("</ul></div>")

    # Pool summary chips
    w("<h2>Pool summary</h2>")
    w("<div>")
    for t in KNOWN_TYPES:
        if agg["by_type"].get(t, 0) > 0:
            w(f'<span class="chip">{t}<b>{agg["by_type"][t]}</b></span>')
    w("</div><div style='margin-top:6px'>")
    for c in KNOWN_COLORS:
        w(f'<span class="chip">{color_swatch(c)}<b>{agg["by_color"][c]}</b></span>')
    if agg["by_color"]["colorless"] > 0:
        w(f'<span class="chip">{color_swatch("colorless")}<b>{agg["by_color"]["colorless"]}</b></span>')
    w("</div><div style='margin-top:6px'>")
    w(f'<span class="chip">single-color<b>{agg["multicolor"]["single"]}</b></span>')
    w(f'<span class="chip">hybrid<b>{agg["multicolor"]["hybrid"]}</b></span>')
    w(f'<span class="chip">colorless<b>{agg["multicolor"]["colorless"]}</b></span>')
    for f in KNOWN_FRAMES:
        n = agg["by_frame"].get(f, 0)
        if n > 0:
            w(f'<span class="chip">{frame_swatch(f)}<b>{n}</b></span>')
    for fa in KNOWN_FACES:
        n = agg["by_face"].get(fa, 0)
        if n > 0:
            w(f'<span class="chip">{face_badge(fa)}<b>{n}</b></span>')
    w("</div>")

    color_rows = list(KNOWN_COLORS)
    if agg["by_color"]["colorless"] > 0:
        color_rows.append("colorless")

    # Color × cost
    w("<h2>Color × total cost</h2>")
    w('<p class="note">Counts of cards in each color at each total-cost bucket. ')
    w("Cells heat-mapped over each row's max (per-color saturation, not global).</p>")
    w('<table class="heat"><thead><tr><th>color</th>')
    for b in COST_BUCKETS:
        w(f'<th class="num">{b}</th>')
    w("<th class=num>total</th></tr></thead><tbody>")
    for c in color_rows:
        row = agg["color_x_cost"][c]
        row_max = max(row[b] for b in COST_BUCKETS)
        w(f'<tr><th>{color_swatch(c)}</th>')
        row_total = 0
        for b in COST_BUCKETS:
            v = row[b]
            row_total += v
            if v > 0:
                w(f'<td class="num" style="{color_cell_style(v, row_max)}">{v}</td>')
            else:
                w('<td class="num"></td>')
        w(f'<td class="num">{row_total}</td></tr>')
    w("</tbody></table>")

    # Color × type
    w("<h2>Color × type</h2>")
    w('<p class="note">Where each color sits across the card-type spectrum. ')
    w("A 0 in a creature column for a color means that color has no creatures yet.</p>")
    w('<table class="heat"><thead><tr><th>color</th>')
    for t in KNOWN_TYPES:
        w(f'<th class="num">{t}</th>')
    w("</tr></thead><tbody>")
    for c in color_rows:
        row = agg["color_x_type"][c]
        # Match Lua: row_max is computed over the rendered columns only, so
        # cards with an unknown type (e.g. `?` for typeless `clear-*` cards)
        # don't inflate the heat scale.
        row_max = max(row.get(t, 0) for t in KNOWN_TYPES)
        w(f'<tr><th>{color_swatch(c)}</th>')
        for t in KNOWN_TYPES:
            v = row[t]
            if v > 0:
                w(f'<td class="num" style="{color_cell_style(v, row_max)}">{v}</td>')
            else:
                w('<td class="num"></td>')
        w("</tr>")
    w("</tbody></table>")

    # Cost-source mix
    w("<h2>Cost-source mix per color</h2>")
    w('<p class="note">Each row sums all cost components from that color\'s cards. ')
    w("Tells you which payment lanes a color leans on.</p>")
    w('<table><thead><tr><th>color</th>')
    for s in KNOWN_SOURCES:
        w(f'<th class="num">{s}</th>')
    w("<th class=num>total</th></tr></thead><tbody>")
    for c in color_rows:
        sm = agg["source_mix"][c]
        total = sm["total"]
        w(f'<tr><th>{color_swatch(c)}</th>')
        for s in KNOWN_SOURCES:
            v = sm.get(s, 0)
            if total > 0 and v > 0:
                pct = round(100 * v / total)
                w(f'<td class="num">{v} <span style="color:var(--text-tertiary);font-size:10px">({pct}%)</span></td>')
            else:
                w('<td class="num"></td>')
        w(f'<td class="num">{total}</td></tr>')
    w("</tbody></table>")

    # Hybrid cards
    w("<h2>Hybrid color cards</h2>")
    w('<p class="note">Cards listing 2+ colors. Grouped by color pair to surface which combinations exist and which don\'t.</p>')
    hybrids: dict[str, dict] = {}
    for card in cards:
        cs = card_colors(card)
        if len(cs) >= 2:
            sorted_colors = sorted(cs)
            key = " + ".join(sorted_colors)
            entry = hybrids.setdefault(key, {"colors": sorted_colors, "cards": []})
            entry["cards"].append(card)
    pair_list = sorted(
        hybrids.items(),
        key=lambda kv: (-len(kv[1]["cards"]), kv[0]),
    )
    if not pair_list:
        w('<p class="note">None.</p>')
    else:
        for key, info in pair_list:
            w('<div class="subtype-row">')
            for i, c in enumerate(info["colors"]):
                if i > 0:
                    w(" ")
                w(color_swatch(c))
            n = len(info["cards"])
            w(f' <span class="cnt">{n} card{"" if n == 1 else "s"}</span> &middot; ')
            w(", ".join(card_tooltip(c) for c in info["cards"]))
            w("</div>")

    # Keywords
    w("<h2>Keyword distribution</h2>")
    w('<p class="note">Cards mentioning each keyword in their abilities text. ')
    w('Substring match (case-insensitive) — flags both intrinsic keywords and references in rider text.</p>')
    kw_rows = sorted(
        ((kw, agg["by_keyword"][kw]) for kw in KNOWN_KEYWORDS if agg["by_keyword"][kw] > 0),
        key=lambda x: -x[1],
    )
    w('<table><thead><tr><th>keyword</th><th class="num">cards</th></tr></thead><tbody>')
    for kw, n in kw_rows:
        w(f'<tr><td>{esc(kw)}</td><td class="num">{n}</td></tr>')
    w("</tbody></table>")

    # Subtype index
    w("<h2>Subtype index</h2>")
    w('<p class="note">Subtypes sorted by card count (descending). ')
    w("Subtypes with only 1-2 cards are candidates for tribal expansion (or removal).</p>")
    subtype_list = sorted(
        (
            {"name": k, "count": len(v["cards"]), "colors": v["colors"], "cards": v["cards"]}
            for k, v in agg["by_subtype"].items()
        ),
        key=lambda x: (-x["count"], x["name"]),
    )
    for st in subtype_list:
        n = st["count"]
        w(
            f'<div class="subtype-row"><b>{esc(st["name"])}</b> '
            f'<span class="cnt">{n} card{"" if n == 1 else "s"}</span> &mdash; '
        )
        for i, c in enumerate(sorted(st["colors"])):
            if i > 0:
                w(" ")
            w(color_swatch(c))
        w(" &middot; ")
        w(", ".join(card_tooltip(c) for c in st["cards"]))
        w("</div>")

    # All cards (+ folded-in typical-turn-played columns when curve is loaded)
    w("<h2>All cards</h2>")
    if curve:
        w(
            f'<p class="note">Every loaded card. Hover an id for name/type/cost/stats/abilities. '
            f'Plays / median / mean / histogram come from {curve.get("n_games", 0)} random-deck '
            f'vs random-deck games (seed {esc(curve.get("seed", "?"))}); both players\' plays count. '
            f'Histogram covers turns 1..{CURVE_MAX_TURN}; '
            f'<span style="font-family:monospace">·</span> marks turns with zero plays. '
            f'Sorted by median turn ascending — never-played cards last.</p>'
        )
        w(
            '<table><thead><tr><th>card</th><th>type</th><th>colors</th>'
            '<th class="num">cost</th><th class="num">x/y</th>'
            '<th class="num">plays</th><th class="num">median</th><th class="num">mean</th>'
            f'<th>turn histogram (1..{CURVE_MAX_TURN})</th></tr></thead><tbody>'
        )
    else:
        w('<p class="note">Every loaded card. Hover an id for name/type/cost/stats/abilities.</p>')
        w(
            '<table><thead><tr><th>card</th><th>type</th><th>colors</th>'
            '<th class="num">cost</th><th class="num">x/y</th></tr></thead><tbody>'
        )
    # Sort: median asc with never-played cards (None) pushed to the
    # bottom, then by id for stable ordering. No curve data → alphabetical.
    if curve:
        sorted_cards = sorted(
            cards,
            key=lambda c: (
                curve_median_turn(curve_for(curve, c.get("id", ""))) or float("inf"),
                c.get("id") or "",
            ),
        )
    else:
        sorted_cards = sorted(cards, key=lambda c: c.get("id") or "")
    for card in sorted_cards:
        cs = card_colors(card)
        color_html = " ".join(color_swatch(c) for c in cs) if cs else color_swatch("colorless")
        if card.get("frame"):
            color_html = f'{color_html} {frame_swatch(card["frame"])}'
        for fa in card.get("face") or []:
            color_html = f'{color_html} {face_badge(fa)}'
        s = card.get("stats")
        xy = f'{s.get("x", 0)}/{s.get("y", 0)}' if s else "—"
        if curve:
            c = curve_for(curve, card.get("id", ""))
            median = curve_median_turn(c)
            mean = curve_mean_turn(c)
            plays = (c or {}).get("plays", 0)
            hist = curve_histogram(c)
            w(
                f'<tr><td>{card_tooltip(card)}</td>'
                f'<td>{esc(card_type(card))}</td>'
                f'<td>{color_html}</td>'
                f'<td class="num">{esc(format_cost(card))}</td>'
                f'<td class="num">{xy}</td>'
                f'<td class="num">{plays if plays else "—"}</td>'
                f'<td class="num">{f"{median:.1f}" if median is not None else "—"}</td>'
                f'<td class="num">{f"{mean:.2f}" if mean is not None else "—"}</td>'
                f'<td style="font-family:monospace;font-size:14px">{esc(hist)}</td></tr>'
            )
        else:
            w(
                f'<tr><td>{card_tooltip(card)}</td>'
                f'<td>{esc(card_type(card))}</td>'
                f'<td>{color_html}</td>'
                f'<td class="num">{esc(format_cost(card))}</td>'
                f'<td class="num">{xy}</td></tr>'
            )
    w("</tbody></table>")

    if decks:
        card_by_id = {c["id"]: c for c in cards if c.get("id")}
        render_archetypes(parts, decks, jmat or [], clusters or [], args.threshold, card_by_id)

    w("</body></html>")
    return "".join(parts)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="tsot pool + archetypes dashboard",
    )
    parser.add_argument("--dir", default="cards", help="cards directory (default: cards)")
    parser.add_argument("--out", default="card-pool.html", help="output HTML path")
    parser.add_argument("--baselines", default="baselines", help="baselines dir for archetypes section")
    parser.add_argument("--champions", default="champions", help="champions dir for archetypes section")
    parser.add_argument("--threshold", type=float, default=0.4,
                        help="Jaccard threshold for clustering (matches `tsot prune-champions` default)")
    parser.add_argument("--no-archetypes", action="store_true",
                        help="skip the deck-archetypes section even if deck JSONs are present")
    args = parser.parse_args()

    cards, load_errors = load_cards(args.dir)
    if not cards:
        sys.stderr.write(f"no cards loaded from {args.dir}\n")
        return 1

    curve = load_curve()
    agg = build_aggregates(cards)

    decks: list[dict] | None = None
    jmat: list[list[float]] | None = None
    clusters: list[list[int]] | None = None
    if not args.no_archetypes:
        loaded = load_decks(args.baselines, args.champions)
        if loaded:
            decks = loaded
            jmat, clusters = cluster_decks(decks, args.threshold)

    html_out = render_html(cards, agg, curve, args, load_errors,
                           decks=decks, jmat=jmat, clusters=clusters)

    Path(args.out).write_text(html_out)
    msg = f"wrote {args.out} ({agg['total']} cards"
    if decks:
        msg += f", {len(decks)} decks, {len(clusters or [])} clusters"
    msg += ")"
    print(msg)
    if load_errors:
        print(f"  {len(load_errors)} load error(s) embedded in the report")
    return 0


if __name__ == "__main__":
    sys.exit(main())
