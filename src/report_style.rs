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
    html! {
        span.card-cell {
            span.card-id { (id) }
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
    }
}
