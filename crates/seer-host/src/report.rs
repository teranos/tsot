// HTML rendering. render_html_report is the entry — it composes the
// verdict banner, commit + CI banner, recent-runs table, before/after
// cards, SVG time series, sampled metric table, and GPU backtrace
// expandables into one self-contained file.

use anyhow::Result;
use std::collections::BTreeMap;

use crate::state::{HostState, Metric, kind_name};
use crate::summary::{RunSummary, env_f64, env_i64};

pub fn render_html_report(
    st: &HostState,
    path: &str,
    history: &[RunSummary],
    current: &RunSummary,
    prior_metrics: &std::collections::BTreeMap<String, Vec<crate::state::Metric>>,
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

    // Backtraces block — legacy per-id detail. Superseded by the
    // aggregation tables above for scanning; kept as ground truth.
    let mut bt_html = String::new();
    for (id, r) in st.gpu_records.iter().take(15) {
        bt_html.push_str(&format!(
            r#"<details><summary>gpu id={id} · {} · {} bytes</summary><pre>{}</pre></details>"#,
            kind_name(r.kind),
            r.size,
            html_escape(&r.backtrace)
        ));
    }

    // Aggregation: heap hotspots grouped by identical backtrace.
    // Same stack = same allocation call site; counting them together
    // turns "480 individual >=1 MB allocations" into "this Rust
    // function allocated N MB across K calls."
    let mut heap_agg: BTreeMap<String, (u32, u64)> = BTreeMap::new();
    for r in st.hotspot_records.values() {
        let e = heap_agg.entry(r.backtrace.clone()).or_insert((0, 0));
        e.0 += 1;
        e.1 += r.size as u64;
    }
    let mut heap_stacks: Vec<(String, u32, u64)> = heap_agg
        .into_iter()
        .map(|(bt, (c, t))| (bt, c, t))
        .collect();
    heap_stacks.sort_by_key(|r| std::cmp::Reverse(r.2));
    let mut heap_stacks_html = String::new();
    for (i, (stack, count, total)) in heap_stacks.iter().take(10).enumerate() {
        let mb = *total as f64 / 1_048_576.0;
        let avg_mb = mb / *count as f64;
        heap_stacks_html.push_str(&format!(
            r#"<details><summary>#{i} · {mb:.2} MB across {count} allocation{s} (avg {avg_mb:.2} MB)</summary><pre>{stack}</pre></details>"#,
            i = i + 1,
            s = if *count == 1 { "" } else { "s" },
            stack = html_escape(stack),
        ));
    }
    if heap_stacks.is_empty() {
        heap_stacks_html.push_str(
            r#"<div class="banner">No heap hotspots captured (no allocations above threshold).</div>"#,
        );
    }

    // Aggregation: GPU resources grouped by (kind, backtrace). Same
    // kind + same stack = same call site emitting the same class of
    // resource. Turns 60 GpuGlobalsBuffer rows into one summary row
    // "buffer · this stack · 60 instances · 3.8 MB".
    let mut gpu_agg: BTreeMap<(u32, String), (u32, u64)> = BTreeMap::new();
    for r in st.gpu_records.values() {
        let e = gpu_agg
            .entry((r.kind, r.backtrace.clone()))
            .or_insert((0, 0));
        e.0 += 1;
        e.1 += r.size as u64;
    }
    let mut gpu_stacks: Vec<(u32, String, u32, u64)> = gpu_agg
        .into_iter()
        .map(|((k, bt), (c, t))| (k, bt, c, t))
        .collect();
    gpu_stacks.sort_by_key(|r| std::cmp::Reverse(r.3));
    let mut gpu_stacks_html = String::new();
    for (i, (kind, stack, count, total)) in gpu_stacks.iter().take(10).enumerate() {
        let mb = *total as f64 / 1_048_576.0;
        let avg_mb = mb / *count as f64;
        gpu_stacks_html.push_str(&format!(
            r#"<details><summary>#{i} · {kname} · {count} instance{s} · {mb:.3} MB total (avg {avg_mb:.3} MB)</summary><pre>{stack}</pre></details>"#,
            i = i + 1,
            kname = kind_name(*kind),
            s = if *count == 1 { "" } else { "s" },
            stack = html_escape(stack),
        ));
    }
    if gpu_stacks.is_empty() {
        gpu_stacks_html
            .push_str(r#"<div class="banner">No GPU allocations captured.</div>"#);
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
        let duration_cell = if h.duration_secs == 0 {
            String::from(r#"<td class="dim">—</td>"#)
        } else {
            format!(r#"<td class="dim">{}</td>"#, fmt_duration(h.duration_secs))
        };
        let status_cell = if h.verdict_passed {
            r#"<td class="down">PASS</td>"#.to_string()
        } else {
            r#"<td class="up">FAIL</td>"#.to_string()
        };
        history_rows.push_str(&format!(
            r#"<tr class="{cls}"><td>{mark}</td><td><a href="{url}">{sha}</a></td><td>{when}</td><td class="{dhc}">{dh}</td><td class="{dgc}">{dgl}</td><td class="{dbc}">{dgb}</td>{ci_cell}{duration_cell}{status_cell}<td class="dim">{leak}</td></tr>"#,
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
            duration_cell = duration_cell,
            status_cell = status_cell,
            leak = leak_marker,
        ));
    }
    let history_html = if history.is_empty() {
        String::from(r#"<div class="banner">No prior history (first run).</div>"#)
    } else {
        format!(
            r#"<table>
    <tr><th></th><th>sha</th><th>when (unix)</th><th>Δheap</th><th>Δgpu live</th><th>Δgpu bytes</th><th>CI</th><th>dur</th><th>verdict</th><th>flags</th></tr>
    {history_rows}
</table>"#
        )
    };

    // Activity log tail — last N high-signal lines from the wasm's
    // seer_emit stream. Filters out per-frame metric spam + per-alloc
    // hotspot detail (each of those has its own dedicated section);
    // keeps setup, tick summaries, inventories, notes, errors,
    // physics player pos, etc. Renders the actual line, not the
    // "seer_emit len=N: " ledger wrapper.
    let noisy_prefixes = [
        "[obs.metric",
        "[obs.hotspots",
        "[obs.hotspot]",
        "[obs.gpu.live]",
        "[obs.gpu.inventory]",
        "[live-buf",
        "[gpu-alloc",
    ];
    let is_signal = |line: &str| -> bool {
        let stripped = line
            .strip_prefix("seer_emit len=")
            .and_then(|s| s.split_once(": "))
            .map(|(_, rest)| rest)
            .unwrap_or(line);
        !noisy_prefixes.iter().any(|p| stripped.starts_with(p))
    };
    let signal_lines: Vec<&String> = st.ledger.iter().filter(|l| is_signal(l)).collect();
    let log_tail_max = 50usize;
    let tail: Vec<&&String> = signal_lines
        .iter()
        .rev()
        .take(log_tail_max)
        .rev()
        .collect();
    let ledger_total = st.ledger.len();
    let log_tail_shown = tail.len();
    let mut log_tail_html = String::new();
    for line in &tail {
        let display = line
            .strip_prefix("seer_emit len=")
            .and_then(|s| s.split_once(": "))
            .map(|(_, rest)| rest)
            .unwrap_or(line.as_str());
        log_tail_html.push_str(&html_escape(display));
        log_tail_html.push('\n');
    }

    // Sacred errors captured from the wasm-side bus during this run.
    // Axiom: never dropped. Highlighted in the report so any
    // Error/Panic surfaces immediately without grepping the log.
    let mut errors_html = String::new();
    if st.errors_captured.is_empty() {
        errors_html.push_str(r#"<div class="banner">No sacred errors captured this run.</div>"#);
    } else {
        errors_html.push_str(r#"<div class="banner leak">"#);
        errors_html.push_str(&format!(
            "<strong>{} error{}:</strong><ul>",
            st.errors_captured.len(),
            if st.errors_captured.len() == 1 { "" } else { "s" }
        ));
        for e in st.errors_captured.iter() {
            errors_html.push_str(&format!("<li><code>{}</code></li>", html_escape(e)));
        }
        errors_html.push_str("</ul></div>");
    }

    // Commit cards — 4-column grid, newest 4 visible by default (last
    // 4 of history, rightmost = current commit). < > buttons swap in
    // older windows of 4 via JS. Each card is tagged data-page from
    // the newest end (page 0 = newest 4, page 1 = next 4 older, etc.).
    // No horizontal scrolling — the current commit is always in the
    // default view without any interaction.
    let n_history = history.len();
    let mut frame_gallery_html = String::new();
    for (i, h) in history.iter().enumerate() {
        let page = if n_history == 0 {
            0
        } else {
            (n_history - 1 - i) / 4
        };
        let frame_url = h.report_url.replace("/report.html", "/frame.png");
        let is_current = h.sha == current.sha && h.when_unix == current.when_unix;
        let cls = if is_current { " current-frame" } else { "" };
        let verdict_tag = if h.verdict_passed {
            r#"<span class="verdict-tag pass">PASS</span>"#
        } else {
            r#"<span class="verdict-tag fail">FAIL</span>"#
        };
        let ci_bit = if h.ci_run_url.is_empty() {
            String::new()
        } else {
            format!(r#"<a href="{}">CI</a>"#, h.ci_run_url)
        };
        let dur_bit = if h.duration_secs > 0 {
            format!(r#"<span>{}</span>"#, fmt_duration(h.duration_secs))
        } else {
            String::new()
        };
        let leak_bit = if h.leak_enabled {
            r#"<span class="tag-leak">leak</span>"#
        } else {
            ""
        };
        // Absolute levels — the current values, not deltas.
        let heap_abs = format!(
            r#"heap: {:.2} MB <span class="delta">({})</span>"#,
            h.heap_end_mb,
            fmt_delta_mb(h.d_heap_mb)
        );
        // wasm size row — includes delta from the prior history entry
        // if available. Zero (unknown) renders as em-dash. Reads at a
        // glance whether this commit made the wasm heavier or lighter.
        let wasm_row = if h.wasm_bytes == 0 {
            String::new()
        } else {
            let mb = h.wasm_bytes as f64 / 1_048_576.0;
            // Find prior non-zero entry.
            let prior_wasm = history
                .iter()
                .rev()
                .find(|p| p.wasm_bytes > 0 && p.when_unix < h.when_unix)
                .map(|p| p.wasm_bytes);
            let delta_html = match prior_wasm {
                Some(p) if p != h.wasm_bytes => {
                    let d = h.wasm_bytes as i64 - p as i64;
                    let d_mb = d.abs() as f64 / 1_048_576.0;
                    let sign = if d > 0 { "+" } else { "-" };
                    let cls = if d > 0 { "up" } else { "down" };
                    format!(r#"<span class="delta {cls}">({sign}{d_mb:.2} MB)</span>"#)
                }
                Some(_) => r#"<span class="delta">(flat)</span>"#.to_string(),
                None => String::new(),
            };
            format!(r#"<div class="card-row">wasm: {mb:.2} MB {delta_html}</div>"#)
        };
        let gpu_abs = format!(
            r#"gpu: {} · {:.2} MB <span class="delta">({} · {})</span>"#,
            h.gpu_live_end,
            h.gpu_bytes_end_mb,
            fmt_delta_count(h.d_gpu_live),
            fmt_delta_mb(h.d_gpu_bytes_mb)
        );
        let sparkline_svg = prior_metrics
            .get(&h.sha)
            .map(|m| render_sparkline_svg(m))
            .unwrap_or_default();
        frame_gallery_html.push_str(&format!(
            r#"<div class="commit-card{cls}" data-page="{page}"><a href="{report}"><img src="{frame}" alt="frame from {sha}" onerror="this.replaceWith(Object.assign(document.createElement('span'),{{textContent:'no frame',className:'delta'}}));" /></a>{sparkline_svg}<div class="card-body"><div class="card-row header"><a class="sha" href="{report}">{sha}</a>{verdict_tag}</div><div class="card-row">{heap_abs}</div><div class="card-row">{gpu_abs}</div>{wasm_row}<div class="card-row footer">{ci_bit}{dur_bit}{leak_bit}</div></div></div>"#,
            report = h.report_url,
            frame = frame_url,
            sha = h.sha,
        ));
    }
    if frame_gallery_html.is_empty() {
        frame_gallery_html.push_str(
            r#"<div class="banner">No commit history yet.</div>"#,
        );
    }

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
  .banner.leak {{ border-left-color: var(--up); background: rgba(248, 113, 113, 0.06); }}
  .banner ul {{ margin: 6px 0 0 0; padding-left: 20px; }}
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
  .frame-row {{ display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; margin: 8px 0; }}
  .gallery-controls {{ display: flex; align-items: center; gap: 8px; margin: 4px 0; }}
  .gallery-controls button {{ background: var(--panel); color: var(--fg); border: 1px solid var(--line); padding: 4px 12px; font-family: inherit; font-size: 14px; cursor: pointer; border-radius: 3px; }}
  .gallery-controls button:hover:not(:disabled) {{ background: var(--line); }}
  .gallery-controls button:disabled {{ opacity: 0.35; cursor: not-allowed; }}
  .page-indicator {{ font-size: 12px; color: var(--dim); }}
  .commit-card {{ background: var(--panel); padding: 10px; border-radius: 4px; display: flex; flex-direction: column; gap: 6px; }}
  .commit-card .sparkline {{ display: block; width: 100%; height: 40px; background: var(--bg); border-radius: 3px; }}
  .commit-card img {{ width: 100%; height: auto; display: block; border-radius: 3px; }}
  .commit-card.current-frame {{ outline: 2px solid var(--accent); }}
  .card-body {{ font-size: 12px; display: flex; flex-direction: column; gap: 3px; }}
  .card-row {{ display: flex; justify-content: space-between; align-items: center; gap: 8px; }}
  .card-row.header {{ font-size: 13px; }}
  .card-row.footer {{ color: var(--dim); font-size: 11px; gap: 12px; justify-content: flex-start; }}
  .verdict-tag {{ font-size: 10px; padding: 2px 6px; border-radius: 3px; letter-spacing: 0.06em; }}
  .verdict-tag.pass {{ background: rgba(34, 197, 94, 0.14); color: var(--down); }}
  .verdict-tag.fail {{ background: rgba(248, 113, 113, 0.14); color: var(--up); }}
  .tag-leak {{ font-size: 10px; padding: 2px 6px; border-radius: 3px; background: rgba(234, 179, 8, 0.14); color: var(--accent3); }}
  .sha {{ font-weight: 600; }}
  svg {{ display: block; background: var(--panel); border-radius: 4px; margin: 8px 0; }}
  .legend {{ display: flex; gap: 20px; font-size: 12px; margin: 4px 0 12px 0; }}
  .swatch {{ display: inline-block; width: 12px; height: 3px; margin-right: 6px; vertical-align: middle; }}
  details {{ background: var(--panel); padding: 8px 12px; margin: 4px 0; border-radius: 3px; }}
  summary {{ cursor: pointer; font-size: 12px; color: var(--dim); }}
  pre {{ font-size: 11px; overflow-x: auto; color: var(--fg); margin: 8px 0 0 0; }}
  pre.log-tail {{ background: var(--panel); padding: 12px 16px; border-radius: 4px; max-height: 400px; overflow-y: auto; white-space: pre-wrap; }}
  code {{ color: var(--accent); }}
</style>
</head><body>
  <h1>seer diagnostic report</h1>
  {verdict_html}
  <div class="banner">
    commit: {sha_link}{branch_html}{ci_banner} · started: <code>{build_ts}</code> · metrics: <code>{n}</code> frames · last frame: <code>{last_frame}</code> · leak: <code>{leak_str}</code>
  </div>

  <h2>Commit history (older → newer)</h2>
  <div class="gallery-controls">
    <button type="button" id="page-older" onclick="pageOlder()" aria-label="older window">&lt;</button>
    <span id="page-indicator" class="page-indicator">newest</span>
    <button type="button" id="page-newer" onclick="pageNewer()" aria-label="newer window">&gt;</button>
  </div>
  <div class="frame-row" id="gallery">
    {frame_gallery_html}
  </div>
  <script>
    (function() {{
      let currentPage = 0;
      const cards = document.querySelectorAll('#gallery .commit-card');
      const pages = Array.from(cards).map(c => parseInt(c.dataset.page || '0', 10));
      const maxPage = pages.length ? Math.max.apply(null, pages) : 0;
      const indicator = document.getElementById('page-indicator');
      const olderBtn = document.getElementById('page-older');
      const newerBtn = document.getElementById('page-newer');
      function apply() {{
        cards.forEach(c => {{
          c.style.display = (parseInt(c.dataset.page || '0', 10) === currentPage) ? '' : 'none';
        }});
        indicator.textContent = currentPage === 0 ? 'newest' : ('page -' + currentPage);
        olderBtn.disabled = currentPage >= maxPage;
        newerBtn.disabled = currentPage <= 0;
      }}
      window.pageOlder = function() {{ if (currentPage < maxPage) {{ currentPage++; apply(); }} }};
      window.pageNewer = function() {{ if (currentPage > 0) {{ currentPage--; apply(); }} }};
      apply();
    }})();
  </script>

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

  <h2>Errors captured this run</h2>
  {errors_html}

  <h2>Activity log (last {log_tail_shown} of {ledger_total} entries, filtered)</h2>
  <pre class="log-tail">{log_tail_html}</pre>

  <h2>Top heap call sites</h2>
  {heap_stacks_html}

  <h2>GPU resources grouped by kind + stack</h2>
  {gpu_stacks_html}

  <h2>GPU allocation call stacks (per-id detail)</h2>
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
        last_frame = last_frame,
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
        frame_gallery_html = frame_gallery_html,
        errors_html = errors_html,
        log_tail_html = log_tail_html,
        log_tail_shown = log_tail_shown,
        ledger_total = ledger_total,
        heap_stacks_html = heap_stacks_html,
        gpu_stacks_html = gpu_stacks_html,
        bt_html = bt_html,
    );

    let _ = before_after_html;
    let _ = history_html;
    let _ = history_rows;

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

/// A run-over-run delta below FLAT_MB_THRESHOLD reads as noise
/// (Bevy internal accounting churn, small vec-growth, allocator
/// alignment). Above it is signal worth investigating. 0.02 MB
/// = 20 KB — coarser than the previous 0.005 MB, so a genuinely
/// steady run reports "flat" instead of "+0.01 MB flicker" noise.
const FLAT_MB_THRESHOLD: f64 = 0.02;

fn fmt_delta_mb(d: f64) -> String {
    if d.abs() < FLAT_MB_THRESHOLD {
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

/// Compact SVG sparkline for one commit's metric time series.
/// Three lines (heap, gpu bytes, gpu live) each normalised to its own
/// max. No axes, no labels — one glance says "flat" or "climbing."
fn render_sparkline_svg(metrics: &[crate::state::Metric]) -> String {
    if metrics.is_empty() {
        return String::new();
    }
    let w = 240.0f32;
    let h = 40.0f32;
    let last_frame = metrics.last().map(|m| m.frame).unwrap_or(1).max(1) as f32;
    let max_heap = metrics.iter().map(|m| m.heap_bytes).max().unwrap_or(1).max(1) as f32;
    let max_gpu_b = metrics.iter().map(|m| m.gpu_bytes).max().unwrap_or(1).max(1) as f32;
    let max_gpu_l = metrics.iter().map(|m| m.gpu_live).max().unwrap_or(1).max(1) as f32;

    let path = |extract: &dyn Fn(&crate::state::Metric) -> f32, max: f32| -> String {
        let mut s = String::new();
        for (i, m) in metrics.iter().enumerate() {
            let x = (m.frame as f32 / last_frame) * w;
            let y = h - (extract(m) / max).clamp(0.0, 1.0) * (h - 2.0) - 1.0;
            if i == 0 {
                s.push_str(&format!("M {x:.1} {y:.1}"));
            } else {
                s.push_str(&format!(" L {x:.1} {y:.1}"));
            }
        }
        s
    };
    let heap = path(&|m| m.heap_bytes as f32, max_heap);
    let gpu_b = path(&|m| m.gpu_bytes as f32, max_gpu_b);
    let gpu_l = path(&|m| m.gpu_live as f32, max_gpu_l);
    // r##"..."## (double-hash delimiter) — `"#` inside the SVG (from
    // `stroke="#22d3ee"`) would otherwise terminate a `r#"..."#`
    // raw string and leave the hex digits floating as bare tokens.
    format!(
        r##"<svg class="sparkline" viewBox="0 0 {w} {h}" preserveAspectRatio="none"><path d="{heap}" stroke="#22d3ee" fill="none" stroke-width="1"/><path d="{gpu_b}" stroke="#f472b6" fill="none" stroke-width="1"/><path d="{gpu_l}" stroke="#eab308" fill="none" stroke-width="1"/></svg>"##,
        w = w as u32,
        h = h as u32,
    )
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m{s:02}s")
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
