// Mirror of RunSummary in crates/seer-host/src/summary.rs and
// CommitReport / Metric / HotspotRecord / GpuRecord in
// crates/seer-host/src/state.rs. Keep these in sync when the Rust
// shape changes — there's no code-gen bridge on purpose (small
// surface, easy to diff).

export interface RunSummary {
  sha: string
  when_unix: number
  frames: number
  first_frame: number
  last_frame: number
  heap_start_mb: number
  heap_end_mb: number
  d_heap_mb: number
  gpu_live_start: number
  gpu_live_end: number
  d_gpu_live: number
  gpu_bytes_start_mb: number
  gpu_bytes_end_mb: number
  d_gpu_bytes_mb: number
  leak_enabled: boolean
  report_url: string
  ci_run_url: string
  verdict_passed: boolean
  verdict_violations: string[]
  duration_secs: number
  wasm_bytes: number
  /** Head-commit message subject line (see summary.rs). Empty on
   * older entries without this field. */
  commit_message: string
}

export interface Metric {
  frame: number
  heap_bytes: number
  gpu_live: number
  gpu_bytes: number
}

export interface HotspotRecord {
  size: number
  align: number
  backtrace: string
}

export interface GpuRecord {
  kind: number   // 1=buffer 2=texture 3=shader
  size: number
  backtrace: string
  /** Added Task 6 — resource name crossed from wasm side to host. */
  label: string
  /** Added Task 7 — ledger index at create time. */
  created_at_seq: number
  /** Added Task 7 — ledger index at destroy time; null if still live at end of run. */
  destroyed_at_seq: number | null
}

export interface CommitReport {
  hotspot_records: Record<string, HotspotRecord>
  gpu_records: Record<string, GpuRecord>
  errors_captured: string[]
  ledger_total: number
  log_tail: string[]
}
