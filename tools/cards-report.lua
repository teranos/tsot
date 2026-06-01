#!/usr/bin/env lua5.4
-- tsot card-pool analytics dashboard.
--
-- Reads every cards/*.lua, aggregates by color / cost / type / subtype,
-- and writes `card-pool.html` — a static dashboard for card-design
-- decisions ("where are the gaps in red", "is purple over-represented
-- at 1-cost", "which subtypes have one card and are basically dead").
--
-- Standalone Lua 5.4 — no Rust, no rebuild, no dependencies. Run from
-- the project root:
--
--   lua5.4 tools/cards-report.lua
--
-- Outputs to ./card-pool.html (override with --out PATH).
--
-- This is a sibling to the Rust HTML reports, NOT a replacement —
-- those need engine state. This one only needs the card source files,
-- so it can run before `cargo build` and surface design issues without
-- spinning up the sim.

local args = {dir = "cards", out = "card-pool.html"}
do
  local i = 1
  while i <= #arg do
    if arg[i] == "--dir" then args.dir = arg[i+1]; i = i + 2
    elseif arg[i] == "--out" then args.out = arg[i+1]; i = i + 2
    elseif arg[i] == "--help" or arg[i] == "-h" then
      print("usage: lua5.4 tools/cards-report.lua [--dir cards] [--out card-pool.html]")
      os.exit(0)
    else
      io.stderr:write("unknown arg: " .. arg[i] .. "\n"); os.exit(1)
    end
  end
end

-- ---------------------------------------------------------------------
-- Load optional turn-curve data (`tsot curve-sample` output)
-- ---------------------------------------------------------------------
-- If `card-curve.json` exists in the cwd, the dashboard adds a typical-
-- turn-played column to the all-cards table and a dedicated section.
-- Generate it by running `tsot curve-sample` first (or `make pool`,
-- which chains them). Absence is fine — the rest of the dashboard
-- renders normally.
--
-- Parsing is regex-line-based, matching the same pattern
-- `tools/archetypes-report.lua` uses for EvolvedDeck JSONs. The
-- producer (`cli_curve_sample.rs`) emits one card per line so this
-- parser stays straightforward — no JSON state machine needed.

local function parse_curve_json(s)
  local out = {n_games = 0, seed = "?", card_curves = {}}
  out.n_games = tonumber(s:match('"n_games"%s*:%s*(%d+)')) or 0
  out.seed = s:match('"seed"%s*:%s*"([^"]+)"') or "?"
  -- Per-card line shape:
  --   "hydra": {"plays": 245, "turns": {"3": 12, "4": 18}}
  for line in s:gmatch("[^\n]+") do
    local card_id, plays, turns_str = line:match(
      '^%s*"([^"]+)"%s*:%s*{%s*"plays"%s*:%s*(%d+)%s*,%s*"turns"%s*:%s*{([^}]*)}'
    )
    if card_id and plays then
      local turns = {}
      for t, c in turns_str:gmatch('"(%d+)"%s*:%s*(%d+)') do
        turns[tonumber(t)] = tonumber(c)
      end
      out.card_curves[card_id] = {plays = tonumber(plays), turns = turns}
    end
  end
  return out
end

local curve_data = nil
do
  local fh = io.open("card-curve.json", "r")
  if fh then
    local s = fh:read("*a")
    fh:close()
    if s and #s > 0 then
      curve_data = parse_curve_json(s)
    end
  end
end

local function curve_for(card_id)
  if not curve_data then return nil end
  return curve_data.card_curves[card_id]
end

-- Median turn (scope-2: both players summed). Returns nil if the
-- card has no recorded plays in this sample.
local function curve_median_turn(c)
  if not c or not c.turns then return nil end
  local list = {}
  for t, count in pairs(c.turns) do
    for _ = 1, count do table.insert(list, t) end
  end
  if #list == 0 then return nil end
  table.sort(list)
  local m = #list // 2
  if #list % 2 == 0 then
    return (list[m] + list[m + 1]) / 2
  else
    return list[m + 1]
  end
end

local function curve_mean_turn(c)
  if not c or not c.turns then return nil end
  local sum, n = 0, 0
  for t, count in pairs(c.turns) do
    sum = sum + t * count
    n = n + count
  end
  if n == 0 then return nil end
  return sum / n
end

-- Render a sparkline like `▁▂▃▄▅▆▇▆▅▃▁` for turns 1..max_turn.
-- `·` (middle-dot) marks a turn with zero plays so the eye can see
-- where the gaps are.
local CURVE_MAX_TURN = 14
local function curve_histogram(c)
  if not c or not c.turns then return "" end
  local blocks = {"▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"}
  local max_count = 0
  for _, n in pairs(c.turns) do
    if n > max_count then max_count = n end
  end
  if max_count == 0 then return "" end
  local parts = {}
  for t = 1, CURVE_MAX_TURN do
    local n = c.turns[t] or 0
    if n == 0 then
      table.insert(parts, "·")
    else
      local idx = math.max(1, math.ceil((n / max_count) * #blocks))
      table.insert(parts, blocks[idx])
    end
  end
  return table.concat(parts)
end

-- ---------------------------------------------------------------------
-- Load cards
-- ---------------------------------------------------------------------

local function list_lua_files(dir)
  local files = {}
  local p = io.popen("ls " .. dir .. "/*.lua 2>/dev/null")
  if not p then return files end
  for line in p:lines() do table.insert(files, line) end
  p:close()
  table.sort(files)
  return files
end

local cards = {}
local load_errors = {}
for _, path in ipairs(list_lua_files(args.dir)) do
  local ok, result = pcall(dofile, path)
  if not ok then
    table.insert(load_errors, {path = path, err = tostring(result)})
  elseif type(result) ~= "table" then
    table.insert(load_errors, {path = path, err = "did not return a table"})
  else
    result._path = path
    table.insert(cards, result)
  end
end

if #cards == 0 then
  io.stderr:write("no cards loaded from " .. args.dir .. "\n")
  os.exit(1)
end

-- ---------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------

local function html_escape(s)
  s = tostring(s or "")
  s = s:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;")
       :gsub('"', "&quot;"):gsub("'", "&#39;")
  return s
end

local function card_total_cost(card)
  -- Returns numeric total, or "X" if any component is variable.
  if not card.cost or #card.cost == 0 then return 0 end
  local total = 0
  local has_x = false
  for _, c in ipairs(card.cost) do
    if c.is_x then has_x = true
    elseif c.amount then total = total + c.amount end
  end
  if has_x then return "X" end
  return total
end

local function format_cost(card)
  if not card.cost or #card.cost == 0 then return "—" end
  local parts = {}
  for _, c in ipairs(card.cost) do
    local amt = c.is_x and "X" or tostring(c.amount or 0)
    table.insert(parts, amt .. " " .. (c.source or "?"))
  end
  return table.concat(parts, " + ")
end

local function card_colors(card)
  return card.colors or {}
end

local function card_subtypes(card)
  return card.subtypes or {}
end

local function card_type(card)
  -- "spell" is a legacy Lua-source alias for "sorcery". The Rust parser
  -- (src/card.rs::parse_type) maps both to (CardType::Spell, Timing::Sorcery);
  -- fold here so the dashboard doesn't expose a distinction that doesn't
  -- exist at runtime.
  local t = card.type
  if t == "spell" then return "sorcery" end
  return t or "?"
end

local function has_keyword(card, kw)
  if not card.abilities then return false end
  for _, a in ipairs(card.abilities) do
    if type(a) == "string" and a:lower():find(kw, 1, true) then return true end
  end
  return false
end

local KNOWN_KEYWORDS = {
  "flying", "reach", "haste", "vigilance",
  "unblockable", "defender", "cannot-block", "first strike",
}

-- Order matters: rendered top-to-bottom in matrices. Current shipping
-- colors first, then planned-but-unshipped (will show as empty rows,
-- which is the point — surfaces the design TODO).
local KNOWN_COLORS = {
  "red", "blue", "green", "purple", "black", "white",
  "pink", "orange", "azure", "transparent", "glow",
}
local KNOWN_TYPES = {"creature", "instant", "sorcery", "artifact", "mutation"}
local KNOWN_SOURCES = {"hand", "mill", "graveyard", "sacrifice", "self"}

-- ---------------------------------------------------------------------
-- Aggregates
-- ---------------------------------------------------------------------

local agg = {
  total = #cards,
  by_color = {},      -- color → count
  by_type = {},       -- type → count
  by_subtype = {},    -- subtype → {cards = {refs}, colors = set}
  by_keyword = {},    -- keyword → count
  color_x_cost = {},  -- color → cost-bucket → count
  color_x_type = {},  -- color → type → count
  source_mix = {},    -- color → source → component count
  multicolor = {},    -- "single" / "hybrid" / "colorless"
  cost_buckets = {0, 1, 2, 3, 4, 5, "6+", "X"},
}

for _, c in ipairs(KNOWN_COLORS) do
  agg.by_color[c] = 0
  agg.color_x_cost[c] = {}
  agg.color_x_type[c] = {}
  agg.source_mix[c] = {total = 0}
  for _, b in ipairs(agg.cost_buckets) do agg.color_x_cost[c][b] = 0 end
  for _, t in ipairs(KNOWN_TYPES) do agg.color_x_type[c][t] = 0 end
  for _, s in ipairs(KNOWN_SOURCES) do agg.source_mix[c][s] = 0 end
end
agg.by_color["colorless"] = 0
agg.color_x_cost["colorless"] = {}
agg.color_x_type["colorless"] = {}
agg.source_mix["colorless"] = {total = 0}
for _, b in ipairs(agg.cost_buckets) do agg.color_x_cost["colorless"][b] = 0 end
for _, t in ipairs(KNOWN_TYPES) do agg.color_x_type["colorless"][t] = 0 end
for _, s in ipairs(KNOWN_SOURCES) do agg.source_mix["colorless"][s] = 0 end

agg.multicolor.single = 0
agg.multicolor.hybrid = 0
agg.multicolor.colorless = 0

for _, kw in ipairs(KNOWN_KEYWORDS) do agg.by_keyword[kw] = 0 end

local function bucket_cost(tc)
  if tc == "X" then return "X" end
  if type(tc) == "number" then
    if tc >= 6 then return "6+" end
    return tc
  end
  return 0
end

for _, card in ipairs(cards) do
  -- type
  local t = card_type(card)
  agg.by_type[t] = (agg.by_type[t] or 0) + 1

  -- colors
  local cs = card_colors(card)
  if #cs == 0 then
    agg.by_color["colorless"] = agg.by_color["colorless"] + 1
    agg.multicolor.colorless = agg.multicolor.colorless + 1
  elseif #cs == 1 then
    agg.multicolor.single = agg.multicolor.single + 1
  else
    agg.multicolor.hybrid = agg.multicolor.hybrid + 1
  end
  for _, color in ipairs(cs) do
    if agg.by_color[color] == nil then agg.by_color[color] = 0 end
    agg.by_color[color] = agg.by_color[color] + 1
  end

  -- color × cost
  local tc = card_total_cost(card)
  local bucket = bucket_cost(tc)
  local cost_targets = #cs == 0 and {"colorless"} or cs
  for _, color in ipairs(cost_targets) do
    if not agg.color_x_cost[color] then
      agg.color_x_cost[color] = {}
      for _, b in ipairs(agg.cost_buckets) do agg.color_x_cost[color][b] = 0 end
    end
    agg.color_x_cost[color][bucket] = (agg.color_x_cost[color][bucket] or 0) + 1
  end

  -- color × type
  for _, color in ipairs(cost_targets) do
    if not agg.color_x_type[color] then
      agg.color_x_type[color] = {}
      for _, tt in ipairs(KNOWN_TYPES) do agg.color_x_type[color][tt] = 0 end
    end
    agg.color_x_type[color][t] = (agg.color_x_type[color][t] or 0) + 1
  end

  -- cost source mix per color
  if card.cost then
    for _, comp in ipairs(card.cost) do
      local src = comp.source or "?"
      local amt = comp.is_x and 1 or (comp.amount or 0)
      for _, color in ipairs(cost_targets) do
        if not agg.source_mix[color] then
          agg.source_mix[color] = {total = 0}
          for _, s in ipairs(KNOWN_SOURCES) do agg.source_mix[color][s] = 0 end
        end
        agg.source_mix[color][src] = (agg.source_mix[color][src] or 0) + amt
        agg.source_mix[color].total = agg.source_mix[color].total + amt
      end
    end
  end

  -- subtypes
  for _, st in ipairs(card_subtypes(card)) do
    local key = st:lower()
    if not agg.by_subtype[key] then
      agg.by_subtype[key] = {cards = {}, colors = {}}
    end
    table.insert(agg.by_subtype[key].cards, card)
    for _, color in ipairs(cs) do agg.by_subtype[key].colors[color] = true end
  end

  -- keywords
  for _, kw in ipairs(KNOWN_KEYWORDS) do
    if has_keyword(card, kw) then
      agg.by_keyword[kw] = agg.by_keyword[kw] + 1
    end
  end
end

-- ---------------------------------------------------------------------
-- HTML rendering
-- ---------------------------------------------------------------------

local function color_cell_style(value, max)
  if max == 0 then return "" end
  local t = value / max
  -- Green accent on dark.
  local r = math.floor((1 - t) * 28 + 24)
  local g = math.floor(t * 160 + 30)
  local b = math.floor((1 - t) * 28 + 24)
  return string.format("background: rgb(%d,%d,%d); color: #eee;", r, g, b)
end

local function color_swatch(color)
  -- Shipping colors get solid swatches. Planned colors get a distinctive
  -- look so they read as "not yet realized" — transparent uses a
  -- checkerboard, glow uses a yellow-green with a box-shadow halo.
  local swatches = {
    red = "#d4604e", blue = "#5d8ec4", green = "#6fa86a",
    purple = "#9a6bbd", black = "#3a3a3a", white = "#d6d4c8",
    colorless = "#86878a",
    pink = "#d97ea8", orange = "#d9885a", azure = "#5ec4d4",
    glow = "#c8e88a",
  }
  local style
  if color == "transparent" then
    style = "background:repeating-conic-gradient(#444 0% 25%, #222 0% 50%) 50% / 6px 6px;"
  elseif color == "glow" then
    style = "background:" .. swatches.glow .. ";box-shadow:0 0 4px " .. swatches.glow .. ";"
  else
    local hex = swatches[color] or "#888"
    style = "background:" .. hex .. ";"
  end
  return string.format(
    '<span style="display:inline-block;width:10px;height:10px;%sborder-radius:2px;margin-right:4px;vertical-align:middle;"></span>%s',
    style, html_escape(color)
  )
end

local css = [[
:root {
  --bg-page: #1a1b1a;
  --bg-panel: #252625;
  --bg-panel-alt: #2e2f2e;
  --bg-row-hover: #2a2b2a;
  --border: #3f4140;
  --text: #dfe1e0;
  --text-secondary: #a9abaa;
  --text-tertiary: #868787;
  --text-emphasis: #fefffe;
  --accent: #7dba8a;
}
* { box-sizing: border-box; }
body {
  font-family: 'JetBrains Mono', 'SF Mono', Monaco, 'Fira Code', Consolas, monospace;
  background: var(--bg-page); color: var(--text); max-width: 1200px;
  margin: 2em auto; padding: 0 1.5em 4em; font-size: 13px; line-height: 1.5;
}
h1 { font-size: 24px; margin: 0 0 0.5em; color: var(--text-emphasis); }
h2 { font-size: 16px; margin: 2em 0 0.5em; padding-bottom: 4px;
     border-bottom: 1px solid var(--border); color: var(--accent); }
p.note { color: var(--text-secondary); font-size: 12px; margin: 0 0 1em; }
.meta { display: flex; gap: 1.5em; margin-bottom: 1em; }
.meta div { color: var(--text-secondary); }
.meta b { color: var(--text-emphasis); margin-left: 6px; }
table { border-collapse: collapse; margin: 0.5em 0; font-size: 12px; }
th, td { padding: 4px 10px; border: 1px solid var(--border); text-align: left; }
th { background: var(--bg-panel-alt); color: var(--text-secondary);
     text-transform: uppercase; font-size: 10px; letter-spacing: 1px; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
tbody tr:hover { background: var(--bg-row-hover); }
.heat td.num { font-weight: 500; }
.chip {
  display: inline-block; padding: 2px 8px; margin: 0 4px 4px 0;
  background: var(--bg-panel); border: 1px solid var(--border);
  border-radius: 12px; font-size: 11px;
}
.chip b { color: var(--accent); margin-left: 4px; }
.card-cell { position: relative; display: inline-block; cursor: help; }
.card-cell .card-tooltip {
  display: none; position: absolute; left: 100%; top: 0; z-index: 50;
  min-width: 320px; max-width: 480px; margin-left: 8px; padding: 12px 16px;
  background: #1a1b1a; color: var(--text); border: 1px solid var(--border);
  border-radius: 7px; box-shadow: 0 4px 16px rgba(0,0,0,0.4);
  font-family: inherit; font-size: 12px; line-height: 1.5; white-space: normal;
  word-break: break-word; overflow-wrap: break-word; pointer-events: none;
  text-align: left; text-transform: none;
}
.card-cell:hover .card-tooltip, .card-cell:focus-within .card-tooltip { display: block; }
.ct-name { color: var(--text-emphasis); font-weight: 600; font-size: 14px; margin-bottom: 4px; }
.ct-meta { color: var(--text-secondary); font-size: 10px;
           text-transform: uppercase; letter-spacing: 1px; margin-bottom: 8px; }
.ct-cost, .ct-stats { color: var(--accent); font-size: 11px; margin-bottom: 4px; }
.ct-abilities { margin-top: 8px; padding-top: 8px; border-top: 1px solid var(--border); }
.ct-abilities div { margin-bottom: 4px; }
.ct-flavor { margin-top: 8px; padding-top: 8px; border-top: 1px dashed var(--border);
             color: var(--text-secondary); font-style: italic; font-size: 11px; }
.subtype-row { padding: 4px 0; border-bottom: 1px dotted var(--border); }
.subtype-row b { color: var(--text-emphasis); }
.subtype-row span.cnt { color: var(--text-tertiary); margin-left: 6px; font-size: 11px; }
.errors { background: #3a2222; border: 1px solid #6a3a3a; padding: 8px 12px;
          border-radius: 4px; margin-bottom: 1em; }
.errors b { color: #d4604e; }
]]

local function card_tooltip(card)
  local name = (card.name and #card.name > 0) and card.name or card.id
  local meta_parts = {}
  if card.colors and #card.colors > 0 then
    table.insert(meta_parts, table.concat(card.colors, " "))
  end
  table.insert(meta_parts, card_type(card))
  if card.subtypes and #card.subtypes > 0 then
    table.insert(meta_parts, "— " .. table.concat(card.subtypes, "/"))
  end
  local meta = table.concat(meta_parts, " ")

  local html_parts = {
    string.format('<span class="card-cell"><span class="card-id">%s</span>',
                  html_escape(card.id)),
    '<span class="card-tooltip">',
    string.format('<div class="ct-name">%s</div>', html_escape(name)),
  }
  if #meta > 0 then
    table.insert(html_parts, string.format('<div class="ct-meta">%s</div>', html_escape(meta)))
  end
  if card.cost and #card.cost > 0 then
    table.insert(html_parts, string.format('<div class="ct-cost">cost: %s</div>',
                                           html_escape(format_cost(card))))
  end
  if card.stats then
    table.insert(html_parts, string.format('<div class="ct-stats">stats: %d/%d</div>',
                                           card.stats.x or 0, card.stats.y or 0))
  end
  if card.abilities and #card.abilities > 0 then
    table.insert(html_parts, '<div class="ct-abilities">')
    for _, a in ipairs(card.abilities) do
      table.insert(html_parts, string.format('<div>%s</div>', html_escape(a)))
    end
    table.insert(html_parts, '</div>')
  end
  if card.flavor and #card.flavor > 0 then
    table.insert(html_parts, string.format('<div class="ct-flavor">%s</div>',
                                           html_escape(card.flavor)))
  end
  table.insert(html_parts, '</span></span>')
  return table.concat(html_parts)
end

-- Build HTML
local html = {}
local function w(s) table.insert(html, s) end

w("<!DOCTYPE html><html lang=en><head><meta charset=utf-8>")
w("<title>tsot — card pool</title>")
w("<style>" .. css .. "</style>")
w("</head><body>")
w("<h1>tsot — card pool</h1>")
w('<div class="meta">')
w(string.format('<div>cards <b>%d</b></div>', agg.total))
w(string.format('<div>dir <b>%s</b></div>', html_escape(args.dir)))
w('</div>')

if #load_errors > 0 then
  w('<div class="errors"><b>Load errors:</b><ul>')
  for _, e in ipairs(load_errors) do
    w(string.format("<li>%s: %s</li>", html_escape(e.path), html_escape(e.err)))
  end
  w("</ul></div>")
end

-- Pool summary chips
w("<h2>Pool summary</h2>")
w("<div>")
for _, t in ipairs(KNOWN_TYPES) do
  if (agg.by_type[t] or 0) > 0 then
    w(string.format('<span class="chip">%s<b>%d</b></span>', t, agg.by_type[t]))
  end
end
w("</div><div style='margin-top:6px'>")
for _, c in ipairs(KNOWN_COLORS) do
  w(string.format('<span class="chip">%s<b>%d</b></span>', color_swatch(c), agg.by_color[c]))
end
if agg.by_color["colorless"] > 0 then
  w(string.format('<span class="chip">%s<b>%d</b></span>',
                  color_swatch("colorless"), agg.by_color["colorless"]))
end
w("</div><div style='margin-top:6px'>")
w(string.format('<span class="chip">single-color<b>%d</b></span>', agg.multicolor.single))
w(string.format('<span class="chip">hybrid<b>%d</b></span>', agg.multicolor.hybrid))
w(string.format('<span class="chip">colorless<b>%d</b></span>', agg.multicolor.colorless))
w("</div>")

-- Color × cost
w("<h2>Color × total cost</h2>")
w('<p class="note">Counts of cards in each color at each total-cost bucket. ')
w("Cells heat-mapped over each row's max (per-color saturation, not global).</p>")
w('<table class="heat"><thead><tr><th>color</th>')
for _, b in ipairs(agg.cost_buckets) do
  w(string.format('<th class="num">%s</th>', tostring(b)))
end
w("<th class=num>total</th></tr></thead><tbody>")
local color_rows = {}
for _, c in ipairs(KNOWN_COLORS) do table.insert(color_rows, c) end
if agg.by_color["colorless"] > 0 then table.insert(color_rows, "colorless") end
for _, c in ipairs(color_rows) do
  local row_max = 0
  for _, b in ipairs(agg.cost_buckets) do
    if agg.color_x_cost[c][b] > row_max then row_max = agg.color_x_cost[c][b] end
  end
  w(string.format('<tr><th>%s</th>', color_swatch(c)))
  local row_total = 0
  for _, b in ipairs(agg.cost_buckets) do
    local v = agg.color_x_cost[c][b]
    row_total = row_total + v
    if v > 0 then
      w(string.format('<td class="num" style="%s">%d</td>',
                      color_cell_style(v, row_max), v))
    else
      w('<td class="num"></td>')
    end
  end
  w(string.format('<td class="num">%d</td></tr>', row_total))
end
w("</tbody></table>")

-- Color × type
w("<h2>Color × type</h2>")
w('<p class="note">Where each color sits across the card-type spectrum. ')
w("A 0 in a creature column for a color means that color has no creatures yet.</p>")
w('<table class="heat"><thead><tr><th>color</th>')
for _, t in ipairs(KNOWN_TYPES) do w(string.format('<th class="num">%s</th>', t)) end
w("</tr></thead><tbody>")
for _, c in ipairs(color_rows) do
  local row_max = 0
  for _, t in ipairs(KNOWN_TYPES) do
    if agg.color_x_type[c][t] > row_max then row_max = agg.color_x_type[c][t] end
  end
  w(string.format('<tr><th>%s</th>', color_swatch(c)))
  for _, t in ipairs(KNOWN_TYPES) do
    local v = agg.color_x_type[c][t]
    if v > 0 then
      w(string.format('<td class="num" style="%s">%d</td>',
                      color_cell_style(v, row_max), v))
    else
      w('<td class="num"></td>')
    end
  end
  w("</tr>")
end
w("</tbody></table>")

-- Cost-source mix
w("<h2>Cost-source mix per color</h2>")
w('<p class="note">Each row sums all cost components from that color\'s cards. ')
w("Tells you which payment lanes a color leans on.</p>")
w('<table><thead><tr><th>color</th>')
for _, s in ipairs(KNOWN_SOURCES) do w(string.format('<th class="num">%s</th>', s)) end
w("<th class=num>total</th></tr></thead><tbody>")
for _, c in ipairs(color_rows) do
  local total = agg.source_mix[c].total
  w(string.format('<tr><th>%s</th>', color_swatch(c)))
  for _, s in ipairs(KNOWN_SOURCES) do
    local v = agg.source_mix[c][s] or 0
    if total > 0 and v > 0 then
      local pct = math.floor(100 * v / total + 0.5)
      w(string.format('<td class="num">%d <span style="color:var(--text-tertiary);font-size:10px">(%d%%)</span></td>', v, pct))
    else
      w('<td class="num"></td>')
    end
  end
  w(string.format('<td class="num">%d</td></tr>', total))
end
w("</tbody></table>")

-- Hybrid cards
w("<h2>Hybrid color cards</h2>")
w('<p class="note">Cards listing 2+ colors. Grouped by color pair to surface which combinations exist and which don\'t.</p>')
local hybrids = {}
for _, card in ipairs(cards) do
  local cs = card_colors(card)
  if #cs >= 2 then
    local sorted_colors = {}
    for _, c in ipairs(cs) do table.insert(sorted_colors, c) end
    table.sort(sorted_colors)
    local key = table.concat(sorted_colors, " + ")
    if not hybrids[key] then hybrids[key] = {colors = sorted_colors, cards = {}} end
    table.insert(hybrids[key].cards, card)
  end
end
local pair_list = {}
for k, v in pairs(hybrids) do table.insert(pair_list, {key = k, info = v}) end
table.sort(pair_list, function(a, b)
  if #a.info.cards == #b.info.cards then return a.key < b.key end
  return #a.info.cards > #b.info.cards
end)
if #pair_list == 0 then
  w('<p class="note">None.</p>')
else
  for _, p in ipairs(pair_list) do
    w('<div class="subtype-row">')
    for i, c in ipairs(p.info.colors) do
      if i > 1 then w(" ") end
      w(color_swatch(c))
    end
    w(string.format(' <span class="cnt">%d card%s</span> &middot; ',
                    #p.info.cards, #p.info.cards == 1 and "" or "s"))
    local names = {}
    for _, c in ipairs(p.info.cards) do table.insert(names, card_tooltip(c)) end
    w(table.concat(names, ", "))
    w("</div>")
  end
end

-- Keywords
w("<h2>Keyword distribution</h2>")
w('<p class="note">Cards mentioning each keyword in their abilities text. ')
w('Substring match (case-insensitive) — flags both intrinsic keywords and references in rider text.</p>')
local kw_rows = {}
for _, kw in ipairs(KNOWN_KEYWORDS) do
  if agg.by_keyword[kw] > 0 then table.insert(kw_rows, {kw, agg.by_keyword[kw]}) end
end
table.sort(kw_rows, function(a, b) return a[2] > b[2] end)
w('<table><thead><tr><th>keyword</th><th class="num">cards</th></tr></thead><tbody>')
for _, r in ipairs(kw_rows) do
  w(string.format('<tr><td>%s</td><td class="num">%d</td></tr>',
                  html_escape(r[1]), r[2]))
end
w("</tbody></table>")

-- Subtype index
w("<h2>Subtype index</h2>")
w('<p class="note">Subtypes sorted by card count (descending). ')
w("Subtypes with only 1-2 cards are candidates for tribal expansion (or removal).</p>")
local subtype_list = {}
for k, v in pairs(agg.by_subtype) do
  table.insert(subtype_list, {name = k, count = #v.cards, colors = v.colors, cards = v.cards})
end
table.sort(subtype_list, function(a, b)
  if a.count == b.count then return a.name < b.name end
  return a.count > b.count
end)
for _, st in ipairs(subtype_list) do
  w(string.format('<div class="subtype-row"><b>%s</b> <span class="cnt">%d card%s</span> &mdash; ',
                  html_escape(st.name), st.count, st.count == 1 and "" or "s"))
  local color_list = {}
  for c, _ in pairs(st.colors) do table.insert(color_list, c) end
  table.sort(color_list)
  for i, c in ipairs(color_list) do
    if i > 1 then w(" ") end
    w(color_swatch(c))
  end
  w(" &middot; ")
  local card_names = {}
  for _, c in ipairs(st.cards) do
    table.insert(card_names, card_tooltip(c))
  end
  w(table.concat(card_names, ", "))
  w("</div>")
end

-- Typical-turn-played section (only when curve-sample data is loaded).
if curve_data then
  w("<h2>Typical turn played</h2>")
  w(string.format(
    '<p class="note">Per-card distribution of the turn on which the card got played, aggregated across %d random-deck vs random-deck games (seed %s). Both players\' plays count. Histogram covers turns 1..%d; <span style="font-family:monospace">·</span> marks turns with zero plays. Sorted by median turn (early-curve first).</p>',
    curve_data.n_games or 0,
    html_escape(tostring(curve_data.seed or "?")),
    CURVE_MAX_TURN))
  -- Build rows: card-id + (curve | nil). Only rows with sampled
  -- plays appear here; the all-cards table below shows every card
  -- with curve data interpolated when available.
  local rows = {}
  for _, card in ipairs(cards) do
    local c = curve_for(card.id)
    if c and c.plays and c.plays > 0 then
      table.insert(rows, {card = card, curve = c})
    end
  end
  table.sort(rows, function(a, b)
    local ma = curve_median_turn(a.curve) or 99
    local mb = curve_median_turn(b.curve) or 99
    if ma ~= mb then return ma < mb end
    return (a.card.id or "") < (b.card.id or "")
  end)
  w('<table><thead><tr>')
  w('<th>card</th><th>cost</th><th class="num">plays</th><th class="num">median</th><th class="num">mean</th><th>turn histogram (1..' .. CURVE_MAX_TURN .. ')</th>')
  w('</tr></thead><tbody>')
  for _, row in ipairs(rows) do
    local median = curve_median_turn(row.curve)
    local mean = curve_mean_turn(row.curve)
    w(string.format(
      '<tr><td>%s</td><td>%s</td><td class="num">%d</td><td class="num">%s</td><td class="num">%s</td><td style="font-family:monospace;font-size:14px">%s</td></tr>',
      card_tooltip(row.card),
      html_escape(format_cost(row.card)),
      row.curve.plays,
      median and string.format("%.1f", median) or "—",
      mean and string.format("%.2f", mean) or "—",
      html_escape(curve_histogram(row.curve))
    ))
  end
  w('</tbody></table>')
end

-- Per-card grid
w("<h2>All cards</h2>")
w('<p class="note">Sortable view of every loaded card. Hover any id to see name/type/cost/stats/abilities.</p>')
if curve_data then
  w('<table><thead><tr><th>card</th><th>type</th><th>colors</th><th class="num">cost</th><th class="num">x/y</th><th class="num">median turn</th><th>curve</th></tr></thead><tbody>')
else
  w('<table><thead><tr><th>card</th><th>type</th><th>colors</th><th class="num">cost</th><th class="num">x/y</th></tr></thead><tbody>')
end
table.sort(cards, function(a, b) return (a.id or "") < (b.id or "") end)
for _, card in ipairs(cards) do
  local cs = card_colors(card)
  local color_html_parts = {}
  for _, c in ipairs(cs) do table.insert(color_html_parts, color_swatch(c)) end
  if #cs == 0 then table.insert(color_html_parts, color_swatch("colorless")) end
  local xy = card.stats and string.format("%d/%d", card.stats.x or 0, card.stats.y or 0) or "—"
  if curve_data then
    local c = curve_for(card.id)
    local median = curve_median_turn(c)
    local hist = curve_histogram(c)
    w(string.format(
      '<tr><td>%s</td><td>%s</td><td>%s</td><td class="num">%s</td><td class="num">%s</td><td class="num">%s</td><td style="font-family:monospace;font-size:14px">%s</td></tr>',
      card_tooltip(card),
      html_escape(card_type(card)),
      table.concat(color_html_parts, " "),
      html_escape(format_cost(card)),
      xy,
      median and string.format("%.1f", median) or "—",
      html_escape(hist)
    ))
  else
    w(string.format(
      '<tr><td>%s</td><td>%s</td><td>%s</td><td class="num">%s</td><td class="num">%s</td></tr>',
      card_tooltip(card),
      html_escape(card_type(card)),
      table.concat(color_html_parts, " "),
      html_escape(format_cost(card)),
      xy
    ))
  end
end
w("</tbody></table>")

w("</body></html>")

local fh, err = io.open(args.out, "w")
if not fh then
  io.stderr:write("could not open " .. args.out .. ": " .. tostring(err) .. "\n")
  os.exit(1)
end
fh:write(table.concat(html))
fh:close()

print(string.format("wrote %s (%d cards)", args.out, agg.total))
if #load_errors > 0 then
  print(string.format("  %d load error(s) embedded in the report", #load_errors))
end
