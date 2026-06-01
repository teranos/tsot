//! Shared dark-mono CSS for HTML reports (matchup-evolved,
//! champions-report, evolve-report) and the `card_cell` widget that
//! every report uses to wrap a card id with a hover tooltip showing
//! the card's name, type, cost, stats, abilities, and flavor.
//!
//! Originally lived in report.rs alongside the legacy variant matchup
//! HTML; the tooltip widget was lost when that file was deleted and
//! re-introduced here so the three current reports can share it.

use maud::{html, Markup};
use tsot::card::{Card, CardType, CostComponent, CostSource};

// TODO(report-css-extraction): the CSS lives inline as a Rust string
// constant, which forces a recompile for any tweak and duplicates the
// styling between this file and the Lua dashboards
// (tools/cards-report.lua, tools/archetypes-report.lua). The mini-card
// widget added 2026-05-31 made the awkwardness obvious. Move to a
// shared `tools/report.css` that both Rust and Lua read at report-write
// time — single source of truth, no rebuild needed for visual tweaks.
// Trade-off: reports stop being single-file portable artifacts, but
// they already aren't (they reference the CSS in <style>), so it's
// a wash. Defer until the CSS is large enough that maintenance pain
// outweighs the migration cost.
pub(crate) const CSS: &str = r#"
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
  --accent-dim: #5a9a6a;
  --player-a: #7eb8da;
  --player-b: #d4a87e;
}
* { box-sizing: border-box; }
body {
  font-family: 'JetBrains Mono', 'SF Mono', Monaco, 'Fira Code', Consolas, monospace;
  background: var(--bg-page);
  color: var(--text);
  max-width: 1100px;
  margin: 2em auto;
  padding: 0 1.5em 4em;
  font-size: 13px;
  line-height: 1.45;
}
h1 {
  color: var(--text-emphasis);
  border-bottom: 1px solid var(--border);
  padding-bottom: 0.4em;
  font-size: 22px;
  font-weight: 600;
}
h2 {
  margin-top: 2.2em;
  color: var(--text-emphasis);
  font-size: 14px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1px;
  opacity: 0.85;
}
.meta {
  display: flex;
  gap: 2em;
  flex-wrap: wrap;
  margin: 1em 0;
  padding: 0.8em 1em;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 3px;
}
.meta .k {
  color: var(--text-tertiary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  margin-right: 0.5em;
}
.meta b { color: var(--accent); font-weight: 600; }
.note {
  color: var(--text-tertiary);
  font-size: 11px;
  margin: 0.4em 0;
}
.panel {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 3px;
  padding: 1em 1.2em;
  margin: 0.5em 0 1.5em;
}
.stat-row {
  display: flex;
  gap: 2em;
  flex-wrap: wrap;
}
.stat .label {
  color: var(--text-tertiary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
}
.stat b {
  color: var(--accent);
  font-size: 18px;
  font-weight: 600;
}
table { border-collapse: collapse; width: 100%; }
table th, table td { padding: 4px 10px; text-align: left; }
table thead th {
  color: var(--text-tertiary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  font-weight: 600;
  border-bottom: 1px solid var(--border);
  padding-bottom: 6px;
}
table tbody tr:hover { background: var(--bg-row-hover); }
.summary th { color: var(--text-secondary); font-weight: normal; }
.num { text-align: right; font-variant-numeric: tabular-nums; }
.vlabel { color: var(--accent); font-weight: 600; }
.muted { color: var(--text-tertiary); }
.muted .num { color: var(--text-tertiary); }
.ok { color: var(--accent); }
.matchup th, .matchup td {
  text-align: center;
  padding: 8px 10px;
  border: 1px solid var(--border);
  min-width: 64px;
}
.matchup thead th { background: var(--bg-panel-alt); }
.matchup tbody th { background: var(--bg-panel-alt); color: var(--accent); }
.matchup td.empty { color: var(--text-tertiary); background: var(--bg-panel-alt); }
.matchup td .rate { font-size: 13px; font-weight: 600; color: #fff; text-shadow: 0 1px 0 rgba(0,0,0,0.4); }
.matchup td .sub { font-size: 9px; color: rgba(255,255,255,0.7); }

/* Card tooltip — hover the card id to surface name/cost/stats/abilities. */
.card-cell {
  position: relative;
  display: inline-block;
  cursor: help;
}
.card-cell .card-id { display: inline-block; }
/* Inline color marker + symbol annotation on the card-cell widget.
   Lets heatmap rows and tables show identity at a glance without
   relying on the tooltip. */
.card-cell .ci-color {
  display: inline-block;
  width: 5px; height: 11px;
  margin-right: 5px;
  border-radius: 2px;
  vertical-align: -1px;
  background: #888;
}
.card-cell .ci-color.ci-red { background: #d4604e; }
.card-cell .ci-color.ci-blue { background: #5d8ec4; }
.card-cell .ci-color.ci-green { background: #6fa86a; }
.card-cell .ci-color.ci-purple { background: #9a6bbd; }
.card-cell .ci-color.ci-black { background: #3a3a3a; outline: 1px solid #5a5a5a; }
.card-cell .ci-color.ci-white { background: #d6d4c8; }
.card-cell .ci-color.ci-pink { background: #d97ea8; }
.card-cell .ci-color.ci-orange { background: #d9885a; }
.card-cell .ci-color.ci-azure { background: #5ec4d4; }
.card-cell .ci-color.ci-transparent {
  background: repeating-conic-gradient(#444 0% 25%, #222 0% 50%) 50% / 3px 3px;
}
.card-cell .ci-color.ci-glow {
  background: #c8e88a; box-shadow: 0 0 3px #c8e88a;
}
.card-cell .ci-color.ci-colorless { background: #86878a; }
.card-cell .ci-symbols {
  display: inline-block;
  margin-left: 5px;
  color: var(--text-secondary);
  font-size: 11px;
  letter-spacing: 0.08em;
}
.card-cell .card-tooltip {
  display: none;
  position: absolute;
  left: 100%;
  top: 0;
  z-index: 50;
  min-width: 320px;
  max-width: 480px;
  margin-left: 8px;
  padding: 12px 16px;
  background: #1a1b1a;
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 7px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
  font-family: inherit;
  font-size: 12px;
  line-height: 1.5;
  white-space: normal;
  word-break: break-word;
  overflow-wrap: break-word;
  pointer-events: none;
  text-align: left;
  text-transform: none;
}
.card-cell:hover .card-tooltip,
.card-cell:focus-within .card-tooltip { display: block; }
.card-tooltip .ct-name {
  color: var(--text-emphasis);
  font-weight: 600;
  font-size: 14px;
  margin-bottom: 4px;
}
.card-tooltip .ct-meta {
  color: var(--text-secondary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  margin-bottom: 8px;
}
.card-tooltip .ct-cost,
.card-tooltip .ct-stats {
  color: var(--accent);
  font-size: 11px;
  margin-bottom: 4px;
}
.card-tooltip .ct-abilities {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
  color: var(--text);
}
.card-tooltip .ct-abilities div { margin-bottom: 4px; }
.card-tooltip .ct-abilities div:last-child { margin-bottom: 0; }
.card-tooltip .ct-flavor {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px dashed var(--border);
  color: var(--text-secondary);
  font-style: italic;
  font-size: 11px;
}

/* Mini-card widget — compact visual chip showing name, color stripe,
   stats, and a snippet of the first ability. Used by the prune report's
   cluster signature lists and reusable by other archetype-style views. */
.mini-card-row { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.mini-card {
  display: inline-block;
  width: 146px;
  min-height: 70px;
  padding: 5px 7px;
  border-radius: 4px;
  border-left: 3px solid #888;
  background: var(--bg-panel-alt);
  vertical-align: top;
  font-size: 10px;
  line-height: 1.35;
  position: relative;
  cursor: help;
}
.mini-card.mini-red { border-left-color: #d4604e; }
.mini-card.mini-blue { border-left-color: #5d8ec4; }
.mini-card.mini-green { border-left-color: #6fa86a; }
.mini-card.mini-purple { border-left-color: #9a6bbd; }
.mini-card.mini-black { border-left-color: #3a3a3a; background: #1f1f1f; }
.mini-card.mini-white { border-left-color: #d6d4c8; }
.mini-card.mini-colorless { border-left-color: #86878a; }
.mini-card .mc-head {
  display: flex; justify-content: space-between; align-items: baseline;
  gap: 6px; margin-bottom: 2px;
}
.mini-card .mc-name {
  /* Sans-serif for titles — JetBrains Mono Bold renders chunky at
     small sizes. The body of the card stays monospace; titles get a
     cleaner UI font. */
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
  color: var(--text); font-weight: 500; font-size: 11.5px;
  letter-spacing: 0.01em;
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical;
  overflow: hidden;
  flex: 1;
}
.mini-card .mc-stats {
  color: var(--accent); font-size: 10px; font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}
.mini-card .mc-symbols {
  color: var(--text-secondary); font-size: 12px;
  letter-spacing: 0.1em; flex-shrink: 0;
}
.mini-card .mc-cost { color: var(--text-secondary); font-size: 9px; margin-top: 1px; }
.mini-card .mc-text {
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical;
  overflow: hidden;
  color: var(--text-secondary); font-size: 9px; margin-top: 3px;
}
.mini-card .mc-count {
  color: var(--text-tertiary); font-size: 9px; margin-top: 2px;
  display: block; text-align: right;
}
"#;

/// Render a card id wrapped in the hover tooltip widget. If the id is
/// not in the pool (e.g., a card removed from the pool but still
/// present in an old champion JSON), falls back to the bare id with no
/// tooltip — callers can drop the result straight into a `td`/`li`/etc.
pub(crate) fn card_cell(pool: &[Card], id: &str) -> Markup {
    let Some(card) = pool.iter().find(|c| c.id == id) else {
        return html! { (id) };
    };
    let name = if card.name.is_empty() {
        id.to_string()
    } else {
        card.name.clone()
    };
    let meta = format_meta(card);
    let cost = format_cost(&card.cost);
    let stats = card.stats.map(|s| format!("{}/{}", s.x, s.y));
    let color_class = match card.colors.first().map(String::as_str) {
        Some("red") => "ci-red",
        Some("blue") => "ci-blue",
        Some("green") => "ci-green",
        Some("purple") => "ci-purple",
        Some("black") => "ci-black",
        Some("white") => "ci-white",
        Some("pink") => "ci-pink",
        Some("orange") => "ci-orange",
        Some("azure") => "ci-azure",
        Some("transparent") => "ci-transparent",
        Some("glow") => "ci-glow",
        _ => "ci-colorless",
    };
    let symbols_str = if card.symbols.is_empty() {
        None
    } else {
        Some(card.symbols.join(" "))
    };
    html! {
        span.card-cell {
            span class={ "ci-color " (color_class) } {}
            span.card-id { (id) }
            @if let Some(sym) = symbols_str.as_deref() {
                span.ci-symbols { (sym) }
            }
            span.card-tooltip {
                div.ct-name { (name) }
                @if !meta.is_empty() { div.ct-meta { (meta) } }
                @if let Some(cost) = cost { div.ct-cost { "cost: " (cost) } }
                @if let Some(s) = stats { div.ct-stats { "stats: " (s) } }
                @if !card.abilities.is_empty() {
                    div.ct-abilities {
                        @for a in &card.abilities { div { (a) } }
                    }
                }
                @if !card.flavor.is_empty() {
                    div.ct-flavor { (card.flavor) }
                }
            }
        }
    }
}

fn format_meta(card: &Card) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !card.colors.is_empty() {
        parts.push(card.colors.join(" "));
    }
    parts.push(kind_label(card.kind).to_string());
    if !card.subtypes.is_empty() {
        parts.push(format!("— {}", card.subtypes.join("/")));
    }
    parts.join(" ")
}

fn kind_label(k: CardType) -> &'static str {
    match k {
        CardType::Unspecified => "card",
        CardType::Creature => "creature",
        CardType::Spell => "spell",
        CardType::Artifact => "artifact",
        CardType::Environment => "environment",
        CardType::Mutation => "mutation",
    }
}

fn format_cost(cost: &[CostComponent]) -> Option<String> {
    if cost.is_empty() {
        return None;
    }
    let s = cost
        .iter()
        .map(|c| {
            let amt = if c.is_x { "X".to_string() } else { c.amount.to_string() };
            format!("{amt} {}", source_label(c.source))
        })
        .collect::<Vec<_>>()
        .join(" + ");
    Some(s)
}

fn source_label(s: CostSource) -> &'static str {
    match s {
        CostSource::Hand => "hand",
        CostSource::Mill => "mill",
        CostSource::Graveyard => "graveyard",
        CostSource::Sacrifice => "sacrifice",
        CostSource::SelfExile => "self-exile",
        CostSource::Attached => "attached",
    }
}

/// Render a card as a compact mini-card chip — color stripe + name +
/// stats + first ability snippet + a per-cluster `[N/M]` count badge.
/// Uses the first listed color for the stripe; multi-color cards take
/// their first color. Unknown ids fall back to a plain text chip.
/// `in_count`/`total` of `(0, 0)` renders no badge — useful when the
/// caller just wants a card chip without a presence ratio.
pub(crate) fn mini_card(pool: &[Card], id: &str, in_count: usize, total: usize) -> Markup {
    let Some(card) = pool.iter().find(|c| c.id == id) else {
        return html! {
            span.mini-card.mini-colorless {
                span.mc-head {
                    span.mc-name { (id) }
                }
            }
        };
    };
    let color_class = match card.colors.first().map(String::as_str) {
        Some("red") => "mini-red",
        Some("blue") => "mini-blue",
        Some("green") => "mini-green",
        Some("purple") => "mini-purple",
        Some("black") => "mini-black",
        Some("white") => "mini-white",
        _ => "mini-colorless",
    };
    let name = if card.name.is_empty() {
        id.to_string()
    } else {
        card.name.clone()
    };
    let stats = card.stats.map(|s| format!("{}/{}", s.x, s.y));
    let cost = format_cost(&card.cost);
    // Abilities-first-line snippet (handlers often emit one ability per
    // entry; the first line is the headline effect for most cards).
    let snippet = card.abilities.first().cloned().unwrap_or_default();
    let symbols = if card.symbols.is_empty() {
        None
    } else {
        Some(card.symbols.join(" "))
    };
    let show_badge = total > 0;
    html! {
        span class={ "mini-card " (color_class) } title=(id) {
            span.mc-head {
                span.mc-name { (name) }
                @if let Some(sym) = &symbols { span.mc-symbols { (sym) } }
                @if let Some(s) = stats { span.mc-stats { (s) } }
            }
            @if let Some(c) = cost { div.mc-cost { (c) } }
            @if !snippet.is_empty() { div.mc-text { (snippet) } }
            @if show_badge {
                span.mc-count { "[" (in_count) "/" (total) "]" }
            }
        }
    }
}
