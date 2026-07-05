// HTML rendering. render_html_report is the entry — it composes the
// verdict banner, commit + CI banner, recent-runs table, before/after
// cards, SVG time series, sampled metric table, and GPU backtrace
// expandables into one self-contained file.

use anyhow::Result;

use crate::state::{HostState, Metric};
use crate::summary::{RunSummary, env_f64, env_i64};

pub fn render_html_report(
    st: &HostState,
    path: &str,
    history: &[RunSummary],
    current: &RunSummary,
) -> Result<()> {
    let m = &st.metrics;
    let n = m.len();

    // Chart geometry.
    let w: u32 = 1000;
    let h: u32 = 320;
    let pad_l: u32 = 60;
    let pad_r: u32 = 20;
    let pad_t: u32 = 20;
    let pad_b: u32 = 40;
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;

    let (last_frame, max_heap, max_gpu_bytes, max_gpu_live) = if n > 0 {
        let last = m[n - 1].frame.max(1);
        let mh = m.iter().map(|x| x.heap_bytes).max().unwrap_or(1).max(1);
        let mgb = m.iter().map(|x| x.gpu_bytes).max().unwrap_or(1).max(1);
        let mgl = m.iter().map(|x| x.gpu_live).max().unwrap_or(1).max(1);
        (last, mh, mgb, mgl)
    } else {
        (1, 1, 1, 1)
    };

    let x_of = |frame: u32| -> f32 {
        pad_l as f32 + (frame as f32 / last_frame as f32) * plot_w as f32
    };
    let y_of_norm =
        |v: f32| -> f32 { pad_t as f32 + plot_h as f32 - v.clamp(0.0, 1.0) * plot_h as f32 };

    let build_path = |vals: &[(u32, f32)]| -> String {
        if vals.is_empty() {
            return String::new();
        }
        let mut s = String::new();
        for (i, (frame, v)) in vals.iter().enumerate() {
            let x = x_of(*frame);
            let y = y_of_norm(*v);
            if i == 0 {
                s.push_str(&format!("M {x:.1} {y:.1}"));
            } else {
                s.push_str(&format!(" L {x:.1} {y:.1}"));
            }
        }
        s
    };

    let heap_points: Vec<(u32, f32)> = m
        .iter()
        .map(|x| (x.frame, x.heap_bytes as f32 / max_heap as f32))
        .collect();
    let gpu_bytes_points: Vec<(u32, f32)> = m
        .iter()
        .map(|x| (x.frame, x.gpu_bytes as f32 / max_gpu_bytes as f32))
        .collect();
    let gpu_live_points: Vec<(u32, f32)> = m
        .iter()
        .map(|x| (x.frame, x.gpu_live as f32 / max_gpu_live as f32))
        .collect();

    let heap_path = build_path(&heap_points);
    let gpu_bytes_path = build_path(&gpu_bytes_points);
    let gpu_live_path = build_path(&gpu_live_points);

    // Before/after snapshots: first and last metric.
    let before_after_html = if n >= 2 {
        let a = m[0];
        let b = m[n - 1];
        let dh = b.heap_bytes as i64 - a.heap_bytes as i64;
        let dgb = b.gpu_bytes as i64 - a.gpu_bytes as i64;
        let dgl = b.gpu_live as i64 - a.gpu_live as i64;
        format!(
            r#"<div class="grid2">
  <div class="card">
    <h3>BEFORE — frame {af}</h3>
    <div class="stat">heap: {ah:.2} MB</div>
    <div class="stat">gpu live: {agl}</div>
    <div class="stat">gpu bytes: {agb:.2} MB</div>
  </div>
  <div class="card">
    <h3>AFTER — frame {bf}</h3>
    <div class="stat">heap: {bh:.2} MB <span class="delta">({dh_s})</span></div>
    <div class="stat">gpu live: {bgl} <span class="delta">({dgl_s})</span></div>
    <div class="stat">gpu bytes: {bgb:.2} MB <span class="delta">({dgb_s})</span></div>
  </div>
</div>"#,
            af = a.frame,
            ah = a.heap_bytes as f64 / 1_048_576.0,
            agl = a.gpu_live,
            agb = a.gpu_bytes as f64 / 1_048_576.0,
            bf = b.frame,
            bh = b.heap_bytes as f64 / 1_048_576.0,
            dh_s = fmt_delta_bytes(dh),
            bgl = b.gpu_live,
            dgl_s = fmt_delta_count(dgl),
            bgb = b.gpu_bytes as f64 / 1_048_576.0,
            dgb_s = fmt_delta_bytes(dgb),
        )
    } else {
        String::from(r#"<div class="banner">Not enough metrics for before/after.</div>"#)
    };

    // Metric table.
    let mut table_rows = String::new();
    let stride = (n / 20).max(1);
    let mut prev: Option<Metric> = None;
    for (i, met) in m.iter().enumerate() {
        if i.is_multiple_of(stride) || i + 1 == n {
            let dh = prev
                .map(|p| met.heap_bytes as i64 - p.heap_bytes as i64)
                .unwrap_or(0);
            let dgl = prev
                .map(|p| met.gpu_live as i64 - p.gpu_live as i64)
                .unwrap_or(0);
            let dgb = prev
                .map(|p| met.gpu_bytes as i64 - p.gpu_bytes as i64)
                .unwrap_or(0);
            table_rows.push_str(&format!(
                r#"<tr><td>{}</td><td>{:.2}</td><td class="{}">{}</td><td>{}</td><td class="{}">{}</td><td>{:.2}</td><td class="{}">{}</td></tr>"#,
                met.frame,
                met.heap_bytes as f64 / 1_048_576.0,
                delta_class(dh),
                fmt_delta_bytes(dh),
                met.gpu_live,
                delta_class(dgl),
                fmt_delta_count(dgl),
                met.gpu_bytes as f64 / 1_048_576.0,
                delta_class(dgb),
                fmt_delta_bytes(dgb),
            ));
            prev = Some(*met);
        }
    }

    // Backtraces block.
    let mut bt_html = String::new();
    for (id, bt) in st.gpu_backtraces.iter().take(15) {
        bt_html.push_str(&format!(
            r#"<details><summary>gpu id={id}</summary><pre>{}</pre></details>"#,
            html_escape(bt)
        ));
    }

    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_string());
    let short_sha: String = sha.chars().take(7).collect();
    let repo =
        std::env::var("GITHUB_REPOSITORY").unwrap_or_else(|_| "teranos/tsot".to_string());
    let branch = std::env::var("GITHUB_REF_NAME").unwrap_or_default();
    let commit_url = format!("https://github.com/{repo}/commit/{sha}");
    let sha_link = format!(r#"<a href="{commit_url}" title="{sha}">{short_sha}</a>"#);
    let branch_html = if branch.is_empty() {
        String::new()
    } else {
        format!(" · branch: <code>{branch}</code>")
    };
    let build_ts = std::env::var("GITHUB_RUN_STARTED_AT").unwrap_or_else(|_| chrono_like_now());

    // Recent runs table — newest-first, one row per prior run.
    let mut history_rows = String::new();
    let mut rev_hist: Vec<&RunSummary> = history.iter().collect();
    rev_hist.reverse();
    for h in rev_hist.iter().take(15) {
        let is_current = h.sha == current.sha && h.when_unix == current.when_unix;
        let mark = if is_current { "→" } else { " " };
        let leak_marker = if h.leak_enabled { " · leak=on" } else { "" };
        let ci_cell = if h.ci_run_url.is_empty() {
            String::from(r#"<td class="dim">—</td>"#)
        } else {
            format!(r#"<td><a href="{}">run</a></td>"#, h.ci_run_url)
        };
        let status_cell = if h.verdict_passed {
            r#"<td class="down">PASS</td>"#.to_string()
        } else {
            r#"<td class="up">FAIL</td>"#.to_string()
        };
        history_rows.push_str(&format!(
            r#"<tr class="{cls}"><td>{mark}</td><td><a href="{url}">{sha}</a></td><td>{when}</td><td class="{dhc}">{dh}</td><td class="{dgc}">{dgl}</td><td class="{dbc}">{dgb}</td>{ci_cell}{status_cell}<td class="dim">{leak}</td></tr>"#,
            cls = if is_current { "current" } else { "" },
            mark = mark,
            url = h.report_url,
            sha = h.sha,
            when = h.when_unix,
            dhc = delta_class(h.d_heap_mb as i64),
            dh = fmt_delta_mb(h.d_heap_mb),
            dgc = delta_class(h.d_gpu_live),
            dgl = fmt_delta_count(h.d_gpu_live),
            dbc = delta_class(h.d_gpu_bytes_mb as i64),
            dgb = fmt_delta_mb(h.d_gpu_bytes_mb),
            ci_cell = ci_cell,
            status_cell = status_cell,
            leak = leak_marker,
        ));
    }
    let history_html = if history.is_empty() {
        String::from(r#"<div class="banner">No prior history (first run).</div>"#)
    } else {
        format!(
            r#"<table>
    <tr><th></th><th>sha</th><th>when (unix)</th><th>Δheap</th><th>Δgpu live</th><th>Δgpu bytes</th><th>CI</th><th>verdict</th><th>flags</th></tr>
    {history_rows}
</table>"#
        )
    };

    // Verdict banner — always present, colored by outcome.
    let verdict_html = if current.verdict_passed {
        format!(
            r#"<div class="verdict pass"><strong>VERDICT: PASS</strong> · thresholds: Δheap ≤ {:.3} MB, Δgpu_live ≤ {}, Δgpu_bytes ≤ {:.3} MB · this run: Δheap {:.3} MB · Δgpu_live {} · Δgpu_bytes {:.3} MB</div>"#,
            env_f64("SEER_MAX_D_HEAP_MB", 1.0),
            env_i64("SEER_MAX_D_GPU_LIVE", 5),
            env_f64("SEER_MAX_D_GPU_BYTES_MB", 0.5),
            current.d_heap_mb,
            current.d_gpu_live,
            current.d_gpu_bytes_mb,
        )
    } else {
        let list = current
            .verdict_violations
            .iter()
            .map(|v| format!("<li>{}</li>", html_escape(v)))
            .collect::<String>();
        format!(
            r#"<div class="verdict fail"><strong>VERDICT: FAIL</strong><ul>{list}</ul></div>"#
        )
    };

    let ci_banner = if current.ci_run_url.is_empty() {
        String::new()
    } else {
        format!(r#" · CI: <a href="{}">run</a>"#, current.ci_run_url)
    };

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en"><head>
<meta charset="utf-8">
<title>seer diagnostic report</title>
<style>
  :root {{
    --bg: #0a0e14; --fg: #cbd5e1; --dim: #64748b; --line: #334155;
    --panel: #121821; --accent: #22d3ee; --accent2: #f472b6; --accent3: #eab308;
    --up: #f87171; --down: #22c55e; --flat: #64748b;
  }}
  body {{ margin: 0; background: var(--bg); color: var(--fg); font-family: ui-monospace, monospace; padding: 24px; max-width: 1200px; margin: auto; }}
  h1 {{ color: #f1f5f9; font-size: 22px; margin: 4px 0 16px 0; }}
  h2 {{ color: #f1f5f9; font-size: 15px; margin: 32px 0 12px 0; text-transform: uppercase; letter-spacing: 0.06em; }}
  h3 {{ color: var(--dim); font-size: 12px; margin: 0 0 8px 0; letter-spacing: 0.08em; text-transform: uppercase; }}
  .banner {{ background: var(--panel); padding: 12px 16px; border-left: 3px solid var(--accent); font-size: 13px; }}
  .grid2 {{ display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }}
  .card {{ background: var(--panel); padding: 16px; border-radius: 4px; }}
  .stat {{ font-size: 14px; margin: 4px 0; }}
  .delta {{ color: var(--dim); }}
  table {{ border-collapse: collapse; width: 100%; font-size: 12px; }}
  th, td {{ text-align: left; padding: 5px 10px; border-bottom: 1px solid var(--line); }}
  th {{ color: var(--dim); font-weight: normal; text-transform: uppercase; letter-spacing: 0.06em; font-size: 11px; }}
  td.up {{ color: var(--up); }}
  td.down {{ color: var(--down); }}
  td.flat {{ color: var(--flat); }}
  td.dim {{ color: var(--dim); }}
  .verdict {{ padding: 14px 18px; margin: 16px 0; border-radius: 4px; font-size: 14px; }}
  .verdict.pass {{ background: rgba(34, 197, 94, 0.10); border-left: 3px solid var(--down); }}
  .verdict.fail {{ background: rgba(248, 113, 113, 0.10); border-left: 3px solid var(--up); color: #fecaca; }}
  .verdict strong {{ font-size: 15px; }}
  .verdict ul {{ margin: 8px 0 0 0; padding-left: 20px; }}
  tr.current {{ background: rgba(34, 211, 238, 0.08); }}
  tr.current td:first-child {{ color: var(--accent); font-weight: bold; }}
  a {{ color: var(--accent); text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  svg {{ display: block; background: var(--panel); border-radius: 4px; margin: 8px 0; }}
  .legend {{ display: flex; gap: 20px; font-size: 12px; margin: 4px 0 12px 0; }}
  .swatch {{ display: inline-block; width: 12px; height: 3px; margin-right: 6px; vertical-align: middle; }}
  details {{ background: var(--panel); padding: 8px 12px; margin: 4px 0; border-radius: 3px; }}
  summary {{ cursor: pointer; font-size: 12px; color: var(--dim); }}
  pre {{ font-size: 11px; overflow-x: auto; color: var(--fg); margin: 8px 0 0 0; }}
  code {{ color: var(--accent); }}
</style>
</head><body>
  <h1>seer diagnostic report</h1>
  {verdict_html}
  <div class="banner">
    commit: {sha_link}{branch_html}{ci_banner} · started: <code>{build_ts}</code> · metrics: <code>{n}</code> frames · last frame: <code>{last_frame}</code> · leak: <code>{leak_str}</code>
  </div>

  <h2>Rendered scene</h2>
  <div class="banner"><img src="frame.png" alt="rendered scene from seer-native this commit" style="max-width: 512px; border-radius: 4px; display: block;" onerror="this.replaceWith(Object.assign(document.createElement('span'),{{textContent:'frame.png not uploaded yet',className:'delta'}}));" /></div>

  <h2>Recent runs</h2>
  {history_html}

  <h2>Before / After</h2>
  {before_after_html}

  <h2>Time series (each series normalised to its own max)</h2>
  <div class="legend">
    <span><span class="swatch" style="background:#22d3ee"></span>heap bytes (max {mh:.2} MB)</span>
    <span><span class="swatch" style="background:#f472b6"></span>gpu bytes (max {mgb:.2} MB)</span>
    <span><span class="swatch" style="background:#eab308"></span>gpu live count (max {mgl})</span>
  </div>
  <svg width="{w}" height="{h}" viewBox="0 0 {w} {h}">
    <line x1="{pad_l}" y1="{pad_t}" x2="{pad_l}" y2="{plot_bottom}" stroke="#64748b" stroke-width="1"/>
    <line x1="{pad_l}" y1="{plot_bottom}" x2="{plot_right}" y2="{plot_bottom}" stroke="#64748b" stroke-width="1"/>
    <path d="{heap_path}" stroke="#22d3ee" fill="none" stroke-width="1.5"/>
    <path d="{gpu_bytes_path}" stroke="#f472b6" fill="none" stroke-width="1.5"/>
    <path d="{gpu_live_path}" stroke="#eab308" fill="none" stroke-width="1.5"/>
    <text x="{pad_l}" y="{text_below}" fill="#64748b" font-size="11" font-family="ui-monospace,monospace">frame 0</text>
    <text x="{text_right_x}" y="{text_below}" text-anchor="end" fill="#64748b" font-size="11" font-family="ui-monospace,monospace">frame {last_frame}</text>
  </svg>

  <h2>Metric table (sampled)</h2>
  <table>
    <tr><th>frame</th><th>heap MB</th><th>Δheap</th><th>gpu live</th><th>Δlive</th><th>gpu MB</th><th>Δgpu MB</th></tr>
    {table_rows}
  </table>

  <h2>GPU allocation call stacks (from wasmtime WasmBacktrace)</h2>
  {bt_html}
</body></html>
"##,
        sha_link = sha_link,
        branch_html = branch_html,
        ci_banner = ci_banner,
        verdict_html = verdict_html,
        build_ts = build_ts,
        n = n,
        leak_str = if current.leak_enabled { "ON" } else { "off" },
        history_html = history_html,
        last_frame = last_frame,
        before_after_html = before_after_html,
        w = w,
        h = h,
        pad_l = pad_l,
        pad_t = pad_t,
        plot_bottom = pad_t + plot_h,
        plot_right = pad_l + plot_w,
        text_below = pad_t + plot_h + 20,
        text_right_x = pad_l + plot_w,
        heap_path = heap_path,
        gpu_bytes_path = gpu_bytes_path,
        gpu_live_path = gpu_live_path,
        mh = max_heap as f64 / 1_048_576.0,
        mgb = max_gpu_bytes as f64 / 1_048_576.0,
        mgl = max_gpu_live,
        table_rows = table_rows,
        bt_html = bt_html,
    );

    std::fs::write(path, html)?;
    Ok(())
}

fn fmt_delta_bytes(d: i64) -> String {
    if d == 0 {
        "flat".to_string()
    } else {
        let mb = d.abs() as f64 / 1_048_576.0;
        if d > 0 {
            format!("+{mb:.2} MB")
        } else {
            format!("-{mb:.2} MB")
        }
    }
}

fn fmt_delta_mb(d: f64) -> String {
    if d.abs() < 0.005 {
        "flat".to_string()
    } else if d > 0.0 {
        format!("+{d:.2} MB")
    } else {
        format!("{d:.2} MB")
    }
}

fn fmt_delta_count(d: i64) -> String {
    if d == 0 {
        "flat".to_string()
    } else if d > 0 {
        format!("+{d}")
    } else {
        format!("{d}")
    }
}

fn delta_class(d: i64) -> &'static str {
    if d > 0 {
        "up"
    } else if d < 0 {
        "down"
    } else {
        "flat"
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn chrono_like_now() -> String {
    // No chrono dep — timestamp from system_time in unix seconds is fine.
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("unix={}", d.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}
