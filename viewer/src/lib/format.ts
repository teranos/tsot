// Display helpers. Mirror short_sha / fmt_delta_mb / fmt_delta_count
// / fmt_duration semantics from crates/seer-host/src/report.rs so
// numbers read the same across UI and CI logs.

export function shortSha(sha: string): string {
  return sha.slice(0, 7)
}

const FLAT_MB_THRESHOLD = 0.02

export function fmtDeltaMB(d: number): string {
  if (Math.abs(d) < FLAT_MB_THRESHOLD) return 'flat'
  if (d > 0) return `+${d.toFixed(2)} MB`
  return `${d.toFixed(2)} MB`
}

export function fmtDeltaCount(d: number): string {
  if (d === 0) return 'flat'
  if (d > 0) return `+${d}`
  return String(d)
}

export function deltaClass(d: number): 'up' | 'down' | 'flat' {
  if (d > 0) return 'up'
  if (d < 0) return 'down'
  return 'flat'
}

export function fmtDeltaBytes(d: number): string {
  if (d === 0) return 'flat'
  const mb = Math.abs(d) / 1_048_576
  if (d > 0) return `+${mb.toFixed(2)} MB`
  return `-${mb.toFixed(2)} MB`
}

export function fmtDuration(secs: number): string {
  if (secs === 0) return '—'
  if (secs < 60) return `${secs}s`
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return `${m}m${String(s).padStart(2, '0')}s`
}

export function fmtWasmMB(bytes: number): string {
  if (bytes === 0) return '—'
  return `${(bytes / 1_048_576).toFixed(2)} MB`
}

export function timeAgo(unix_secs: number): string {
  if (unix_secs === 0) return ''
  const diff_secs = Date.now() / 1000 - unix_secs
  if (diff_secs < 60) return `${Math.floor(diff_secs)}s ago`
  const mins = Math.floor(diff_secs / 60)
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  const days = Math.floor(hrs / 24)
  return `${days}d ago`
}

export function kindName(kind: number): string {
  if (kind === 1) return 'buffer'
  if (kind === 2) return 'texture'
  if (kind === 3) return 'shader'
  return '?'
}
