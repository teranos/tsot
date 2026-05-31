#!/usr/bin/env lua5.4
-- tsot deck-archetype dashboard.
--
-- Loads every EvolvedDeck JSON in baselines/ and champions/, clusters
-- them by Jaccard similarity on card-id sets, and writes
-- `archetypes-report.html` — a static dashboard showing:
--
--   - cluster roster (1+ decks per cluster, named by representative
--     centroid, with avg internal Jaccard)
--   - per-cluster signature cards (cards that appear in ≥ half the
--     cluster's decks)
--   - full Jaccard heatmap across all decks
--   - per-deck centroid distance + nearest baseline (for unmatched-
--     champion diagnostics — answers "how close is this champion to
--     becoming a curate-cluster member?")
--
-- Designed to make `curate-baselines` output meaningful. After running
-- this, the "10 unmatched champions" line in curate is no longer
-- mysterious — you see exactly which clusters they form and what
-- Jaccard threshold would have matched them.
--
-- Standalone Lua 5.4. Run from project root:
--
--   lua5.4 tools/archetypes-report.lua
--
-- Outputs to ./archetypes-report.html (override with --out PATH).

local args = {
  baselines_dir = "baselines",
  champions_dir = "champions",
  out = "archetypes-report.html",
  threshold = 0.5,
}
do
  local i = 1
  while i <= #arg do
    if arg[i] == "--baselines" then args.baselines_dir = arg[i+1]; i = i + 2
    elseif arg[i] == "--champions" then args.champions_dir = arg[i+1]; i = i + 2
    elseif arg[i] == "--out" then args.out = arg[i+1]; i = i + 2
    elseif arg[i] == "--threshold" then args.threshold = tonumber(arg[i+1]); i = i + 2
    elseif arg[i] == "--help" or arg[i] == "-h" then
      print("usage: lua5.4 tools/archetypes-report.lua [--baselines DIR] [--champions DIR] [--out PATH] [--threshold 0.5]")
      os.exit(0)
    else
      io.stderr:write("unknown arg: " .. arg[i] .. "\n"); os.exit(1)
    end
  end
end

-- ---------------------------------------------------------------------
-- JSON loader (minimal — handles tsot's EvolvedDeck shape)
-- ---------------------------------------------------------------------
-- The deck files are simple: label, fitness, base_seed, generations_run,
-- card_ids[]. No nesting beyond the array, no escaped strings. Avoids
-- pulling a json lib dependency.

local function read_file(path)
  local fh = io.open(path, "r")
  if not fh then return nil end
  local s = fh:read("*a")
  fh:close()
  return s
end

local function parse_deck_json(s)
  local label = s:match('"label"%s*:%s*"([^"]+)"') or "unknown"
  local fitness = tonumber(s:match('"fitness"%s*:%s*([%-%d%.eE]+)')) or 0
  local card_ids = {}
  local arr = s:match('"card_ids"%s*:%s*%[([^%]]+)%]')
  if arr then
    for cid in arr:gmatch('"([^"]+)"') do
      table.insert(card_ids, cid)
    end
  end
  return {label = label, fitness = fitness, card_ids = card_ids}
end

local function list_json_files(dir)
  local files = {}
  local p = io.popen("ls " .. dir .. "/*.json 2>/dev/null")
  if not p then return files end
  for line in p:lines() do table.insert(files, line) end
  p:close()
  table.sort(files)
  return files
end

local function basename(path)
  return path:match("([^/]+)%.json$") or path
end

-- ---------------------------------------------------------------------
-- Load
-- ---------------------------------------------------------------------

local decks = {}  -- array of {name, source, label, fitness, ids_list, ids_set}

local function load_dir(dir, source)
  for _, path in ipairs(list_json_files(dir)) do
    local s = read_file(path)
    if s then
      local d = parse_deck_json(s)
      local id_set = {}
      for _, cid in ipairs(d.card_ids) do id_set[cid] = true end
      table.insert(decks, {
        name = basename(path),
        source = source,
        label = d.label,
        fitness = d.fitness,
        ids_list = d.card_ids,
        ids_set = id_set,
      })
    end
  end
end

load_dir(args.baselines_dir, "baseline")
load_dir(args.champions_dir, "champion")

if #decks == 0 then
  io.stderr:write("no decks loaded\n")
  os.exit(1)
end

-- ---------------------------------------------------------------------
-- Jaccard
-- ---------------------------------------------------------------------

local function set_size(s)
  local n = 0
  for _ in pairs(s) do n = n + 1 end
  return n
end

local function jaccard(a, b)
  local inter, union = 0, 0
  for k in pairs(a) do
    union = union + 1
    if b[k] then inter = inter + 1 end
  end
  for k in pairs(b) do
    if not a[k] then union = union + 1 end
  end
  if union == 0 then return 1.0 end
  return inter / union
end

-- Pairwise matrix.
local n = #decks
local jmat = {}
for i = 1, n do
  jmat[i] = {}
  for j = 1, n do
    if i == j then jmat[i][j] = 1.0
    elseif j < i then jmat[i][j] = jmat[j][i]
    else jmat[i][j] = jaccard(decks[i].ids_set, decks[j].ids_set)
    end
  end
end

-- ---------------------------------------------------------------------
-- Clustering: greedy single-linkage at the threshold.
-- ---------------------------------------------------------------------

local cluster_of = {}  -- deck idx → cluster id
for i = 1, n do cluster_of[i] = i end

local function find_root(i)
  while cluster_of[i] ~= i do
    cluster_of[i] = cluster_of[cluster_of[i]]
    i = cluster_of[i]
  end
  return i
end

for i = 1, n do
  for j = i + 1, n do
    if jmat[i][j] >= args.threshold then
      local ri = find_root(i)
      local rj = find_root(j)
      if ri ~= rj then cluster_of[ri] = rj end
    end
  end
end

-- Bucket decks by cluster root.
local clusters = {}  -- root → list of deck idx
for i = 1, n do
  local r = find_root(i)
  if not clusters[r] then clusters[r] = {} end
  table.insert(clusters[r], i)
end

-- Sort clusters by size descending, then by first member's name.
local cluster_list = {}
for _, idxs in pairs(clusters) do table.insert(cluster_list, idxs) end
table.sort(cluster_list, function(a, b)
  if #a == #b then return decks[a[1]].name < decks[b[1]].name end
  return #a > #b
end)

-- For each cluster, find the centroid (member with highest avg Jaccard
-- to the rest) and the signature cards (≥ ceil(#cluster / 2) appearances).
local function cluster_centroid(idxs)
  if #idxs == 1 then return idxs[1] end
  local best_i, best_avg = idxs[1], -1
  for _, i in ipairs(idxs) do
    local sum, count = 0, 0
    for _, j in ipairs(idxs) do
      if i ~= j then sum = sum + jmat[i][j]; count = count + 1 end
    end
    local avg = count > 0 and sum / count or 0
    if avg > best_avg then best_avg = avg; best_i = i end
  end
  return best_i, best_avg
end

local function cluster_signature(idxs)
  local card_count = {}
  for _, i in ipairs(idxs) do
    for cid in pairs(decks[i].ids_set) do
      card_count[cid] = (card_count[cid] or 0) + 1
    end
  end
  local half = math.ceil(#idxs / 2)
  local sig = {}
  for cid, c in pairs(card_count) do
    if c >= half then table.insert(sig, {id = cid, in_count = c}) end
  end
  table.sort(sig, function(a, b)
    if a.in_count == b.in_count then return a.id < b.id end
    return a.in_count > b.in_count
  end)
  return sig
end

-- For singletons (cluster size 1), surface the nearest non-self deck
-- and its Jaccard. Useful for "this unmatched champion is 0.62 from
-- baseline-eac8-r2" diagnostic.
local function nearest_other(idx)
  local best_j, best_v = nil, -1
  for j = 1, n do
    if j ~= idx and jmat[idx][j] > best_v then
      best_v = jmat[idx][j]
      best_j = j
    end
  end
  return best_j, best_v
end

-- ---------------------------------------------------------------------
-- HTML
-- ---------------------------------------------------------------------

local function html_escape(s)
  s = tostring(s or "")
  s = s:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;")
       :gsub('"', "&quot;"):gsub("'", "&#39;")
  return s
end

local function cell_color(t)
  -- 0 → dark red; 0.5 → bg neutral; 1.0 → bright green.
  t = math.max(0, math.min(1, t))
  local r, g, b
  if t < 0.5 then
    local k = t * 2
    r = math.floor(80 + (1 - k) * 60)
    g = math.floor(60 + k * 40)
    b = math.floor(60)
  else
    local k = (t - 0.5) * 2
    r = math.floor(40 + (1 - k) * 40)
    g = math.floor(120 + k * 50)
    b = math.floor(40)
  end
  return string.format("background: rgb(%d,%d,%d); color: #eee;", r, g, b)
end

local css = [[
:root {
  --bg-page: #1a1b1a;
  --bg-panel: #252625;
  --bg-panel-alt: #2e2f2e;
  --border: #3f4140;
  --text: #dfe1e0;
  --text-secondary: #a9abaa;
  --text-tertiary: #868787;
  --text-emphasis: #fefffe;
  --accent: #7dba8a;
}
* { box-sizing: border-box; }
body {
  font-family: 'JetBrains Mono', 'SF Mono', Monaco, monospace;
  background: var(--bg-page); color: var(--text); max-width: 1300px;
  margin: 2em auto; padding: 0 1.5em 4em; font-size: 13px; line-height: 1.5;
}
h1 { font-size: 24px; margin: 0 0 0.5em; color: var(--text-emphasis); }
h2 { font-size: 16px; margin: 2em 0 0.5em; padding-bottom: 4px;
     border-bottom: 1px solid var(--border); color: var(--accent); }
h3 { font-size: 14px; margin: 1em 0 0.3em; color: var(--text-emphasis); }
p.note { color: var(--text-secondary); font-size: 12px; margin: 0 0 1em; }
.meta { display: flex; gap: 1.5em; margin-bottom: 1em; }
.meta div { color: var(--text-secondary); }
.meta b { color: var(--text-emphasis); margin-left: 6px; }
table { border-collapse: collapse; margin: 0.5em 0; font-size: 12px; }
th, td { padding: 4px 8px; border: 1px solid var(--border); text-align: left; }
th { background: var(--bg-panel-alt); color: var(--text-secondary);
     text-transform: uppercase; font-size: 10px; letter-spacing: 1px; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
tr.singleton td { color: var(--text-tertiary); }
.cluster {
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  padding: 8px 12px;
  margin: 0 0 1em;
  background: var(--bg-panel);
}
.cluster.singleton { border-left-color: var(--text-tertiary); }
.cluster .centroid { color: var(--accent); font-weight: 600; }
.cluster .sigcards { color: var(--text-secondary); font-size: 11px;
                     margin-top: 4px; }
.cluster .sigcards b { color: var(--text); }
.cluster .members { font-size: 11px; color: var(--text-secondary);
                    margin-top: 4px; }
.cluster .members b { color: var(--text); }
.heatmap { font-size: 9px; }
.heatmap th { padding: 2px 6px; }
.heatmap td { padding: 2px 4px; text-align: center; min-width: 32px; }
.source-baseline { color: var(--accent); }
.source-champion { color: var(--text); }
.chip { display: inline-block; padding: 2px 8px; margin: 0 4px 4px 0;
        background: var(--bg-panel); border: 1px solid var(--border);
        border-radius: 12px; font-size: 11px; }
.chip b { color: var(--accent); margin-left: 4px; }
]]

local html_parts = {}
local function w(s) table.insert(html_parts, s) end

w("<!DOCTYPE html><html lang=en><head><meta charset=utf-8>")
w("<title>tsot — deck archetypes</title>")
w("<style>" .. css .. "</style>")
w("</head><body>")
w("<h1>tsot — deck archetypes</h1>")

local n_baseline = 0
local n_champion = 0
for _, d in ipairs(decks) do
  if d.source == "baseline" then n_baseline = n_baseline + 1
  else n_champion = n_champion + 1 end
end

w('<div class="meta">')
w(string.format('<div>decks <b>%d</b></div>', n))
w(string.format('<div>baselines <b>%d</b></div>', n_baseline))
w(string.format('<div>champions <b>%d</b></div>', n_champion))
w(string.format('<div>threshold <b>%.2f</b></div>', args.threshold))
w(string.format('<div>clusters <b>%d</b></div>', #cluster_list))
w('</div>')

-- Cluster roster
w("<h2>Clusters</h2>")
w('<p class="note">Greedy single-linkage at Jaccard ≥ ')
w(string.format('%.2f', args.threshold))
w('. A "singleton" cluster (1 member) means that deck has no Jaccard-similar peer at this threshold — usually a distinct attractor or an outlier.</p>')

local cluster_id = 0
for _, idxs in ipairs(cluster_list) do
  cluster_id = cluster_id + 1
  local size = #idxs
  local is_singleton = size == 1
  local centroid_idx, centroid_avg = cluster_centroid(idxs)
  local sig = cluster_signature(idxs)

  w(is_singleton and '<div class="cluster singleton">' or '<div class="cluster">')
  w(string.format('<h3>Cluster %d &middot; %d deck%s', cluster_id, size, size == 1 and "" or "s"))
  if not is_singleton then
    w(string.format(' &middot; avg internal Jaccard %.2f', centroid_avg or 0))
  end
  w('</h3>')

  -- Centroid + members
  w('<div class="members">')
  w('representative: <span class="centroid">')
  w(html_escape(decks[centroid_idx].name))
  w(' (')
  w(html_escape(decks[centroid_idx].source))
  w(')</span>')
  if not is_singleton then
    w(' &middot; <b>members:</b> ')
    local member_strs = {}
    for _, i in ipairs(idxs) do
      table.insert(member_strs, html_escape(decks[i].name))
    end
    w(table.concat(member_strs, ", "))
  end
  w('</div>')

  -- Signature cards (≥ half the cluster has each)
  if #sig > 0 then
    w('<div class="sigcards"><b>signature cards</b> (in ≥ ½ of cluster): ')
    local sig_strs = {}
    for i, s in ipairs(sig) do
      if i > 18 then break end
      table.insert(sig_strs, string.format('%s [%d/%d]', html_escape(s.id), s.in_count, size))
    end
    w(table.concat(sig_strs, ", "))
    if #sig > 18 then
      w(string.format(' &middot; +%d more', #sig - 18))
    end
    w('</div>')
  end

  -- Singleton diagnostic: nearest other deck + Jaccard
  if is_singleton then
    local nj, nv = nearest_other(idxs[1])
    if nj then
      w(string.format(
        '<div class="sigcards">nearest other deck: <b>%s</b> at Jaccard <b>%.2f</b>',
        html_escape(decks[nj].name), nv
      ))
      if nv >= args.threshold * 0.7 and nv < args.threshold then
        w(string.format(' &middot; <b>would join its cluster at threshold ≤ %.2f</b>', nv))
      end
      w('</div>')
    end
  end

  w('</div>')
end

-- Per-deck table
w("<h2>All decks</h2>")
w('<p class="note">Deck-level view. <em>nearest</em> is the closest other deck by Jaccard — '
  .. 'low values indicate isolated decks worth examining as potential new attractors.</p>')
w('<table><thead><tr><th>deck</th><th>source</th><th>label</th><th class="num">fitness</th>'
  .. '<th>nearest</th><th class="num">jaccard</th><th class="num">cluster</th></tr></thead><tbody>')
-- For each deck, find cluster id
local cluster_id_of = {}
do
  local cid = 0
  for _, idxs in ipairs(cluster_list) do
    cid = cid + 1
    for _, i in ipairs(idxs) do cluster_id_of[i] = cid end
  end
end
for i = 1, n do
  local nj, nv = nearest_other(i)
  local is_singleton = #(clusters[find_root(i)]) == 1
  w(is_singleton and '<tr class="singleton">' or '<tr>')
  w(string.format('<td>%s</td>', html_escape(decks[i].name)))
  w(string.format('<td class="source-%s">%s</td>', decks[i].source, decks[i].source))
  w(string.format('<td>%s</td>', html_escape(decks[i].label)))
  w(string.format('<td class="num">%.3f</td>', decks[i].fitness))
  if nj then
    w(string.format('<td>%s</td>', html_escape(decks[nj].name)))
    w(string.format('<td class="num">%.2f</td>', nv))
  else
    w('<td></td><td class="num"></td>')
  end
  w(string.format('<td class="num">#%d</td>', cluster_id_of[i] or 0))
  w('</tr>')
end
w('</tbody></table>')

-- Heatmap
w("<h2>Jaccard heatmap</h2>")
w('<p class="note">Pairwise Jaccard similarity. Cells colored: red below 0.5, green at 0.5+, '
  .. 'brighter green nearer 1.0.</p>')
w('<table class="heatmap"><thead><tr><th></th>')
for i = 1, n do
  w(string.format('<th>%d</th>', i))
end
w('</tr></thead><tbody>')
for i = 1, n do
  w(string.format('<tr><th>%d %s</th>', i, html_escape(decks[i].name)))
  for j = 1, n do
    local v = jmat[i][j]
    if i == j then
      w(string.format('<td style="background:#3f4140;color:#868787">%.2f</td>', v))
    else
      w(string.format('<td style="%s">%.2f</td>', cell_color(v), v))
    end
  end
  w('</tr>')
end
w('</tbody></table>')

w("</body></html>")

local fh, err = io.open(args.out, "w")
if not fh then
  io.stderr:write("could not open " .. args.out .. ": " .. tostring(err) .. "\n")
  os.exit(1)
end
fh:write(table.concat(html_parts))
fh:close()

print(string.format("wrote %s (%d decks, %d clusters)", args.out, n, #cluster_list))
