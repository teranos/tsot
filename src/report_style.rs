//! Shared dark-mono CSS for HTML reports (matchup-evolved,
//! champions-report). Originally lived in report.rs alongside the
//! variant matchup HTML; extracted here so the legacy report.rs can be
//! deleted without losing the styling.

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
"#;
