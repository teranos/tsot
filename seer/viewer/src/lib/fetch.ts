import type { RunSummary, CommitReport, Metric } from './types'

// Extract the full sha from a history entry. Old (pre-M1) entries
// have a 7-char `sha` field but their per-sha S3 objects still live
// at 40-char paths; the FULL sha is embedded in report_url. Post-M5
// report_url shrinks to `/<sha>/` but the last path segment is still
// the full sha. This helper is where that history is absorbed.
export function shaFromEntry(entry: RunSummary): string {
  let u = entry.report_url
  if (u.endsWith('/report.html')) u = u.slice(0, -'/report.html'.length)
  if (u.endsWith('/')) u = u.slice(0, -1)
  const parts = u.split('/').filter(p => p.length > 0)
  const last = parts[parts.length - 1] || ''
  // If report_url didn't yield a sha (malformed or missing), fall
  // back to the display .sha — degraded but not silently wrong.
  return last || entry.sha
}

async function loadJson<T>(url: string): Promise<T> {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`${url} → HTTP ${res.status}`)
  return res.json() as Promise<T>
}

// Common prefix for every data artifact seer-host produces. Kept
// in one constant so a future flattening (drop /perf/) is a single
// edit here, and so LiveRun composes URLs the same way as JSON
// loaders. Matches S3 upload prefix in seer.yml / seer-browser.yml.
export const DATA_BASE = '/perf'

export function wasmUrl(sha: string): string {
  return `${DATA_BASE}/${sha}/seer.wasm`
}

export function frameUrl(sha: string): string {
  return `${DATA_BASE}/${sha}/frame.png`
}

export async function loadHistory(): Promise<RunSummary[]> {
  return loadJson<RunSummary[]>(`${DATA_BASE}/history.json`)
}

export interface CommitData {
  metrics: Metric[] | null
  metricsError: string
  report: CommitReport | null
  reportError: string
}

// Fetch a commit's per-sha data. Independent try/catch per file so
// one 404 doesn't hide the other file's data. Empty state != error
// state — each field renders differently downstream.
export async function loadCommitData(sha: string): Promise<CommitData> {
  const [metricsRes, reportRes] = await Promise.allSettled([
    loadJson<Metric[]>(`${DATA_BASE}/${sha}/metrics.json`),
    loadJson<CommitReport>(`${DATA_BASE}/${sha}/report.json`),
  ])
  return {
    metrics: metricsRes.status === 'fulfilled' ? metricsRes.value : null,
    metricsError: metricsRes.status === 'rejected' ? String(metricsRes.reason?.message || metricsRes.reason) : '',
    report: reportRes.status === 'fulfilled' ? reportRes.value : null,
    reportError: reportRes.status === 'rejected' ? String(reportRes.reason?.message || reportRes.reason) : '',
  }
}
