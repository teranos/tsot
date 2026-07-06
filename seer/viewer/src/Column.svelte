<script lang="ts">
  import type { RunSummary, CommitReport, Metric } from './lib/types'
  import { loadCommitData, shaFromEntry } from './lib/fetch'
  import ColumnHeader from './ColumnHeader.svelte'
  import DeltasRow from './DeltasRow.svelte'
  import Frame from './Frame.svelte'
  import Sparklines from './Sparklines.svelte'
  import Errors from './Errors.svelte'
  import GpuRecords from './GpuRecords.svelte'
  import Hotspots from './Hotspots.svelte'
  import LogTail from './LogTail.svelte'
  import LiveRun from './LiveRun.svelte'

  let {
    entry,
    current,
    onSelect,
  }: { entry: RunSummary, current: boolean, onSelect: (sha: string) => void } = $props()
  const sha = shaFromEntry(entry)

  let metrics: Metric[] | null = $state(null)
  let metricsError = $state('')
  let report: CommitReport | null = $state(null)
  let reportError = $state('')
  let loading = $state(true)

  $effect(() => {
    loading = true
    loadCommitData(sha).then(d => {
      metrics = d.metrics
      metricsError = d.metricsError
      report = d.report
      reportError = d.reportError
      loading = false
    })
  })
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="col" class:current data-sha={sha} onclick={() => onSelect(sha)}>
  <ColumnHeader {entry} {sha} />
  <div class="body">
    <Frame {sha} />
    <DeltasRow {entry} />
    <Sparklines {metrics} error={metricsError} />
    <Errors errors={report?.errors_captured ?? []} {loading} error={reportError} />
    <GpuRecords records={report?.gpu_records ?? {}} {loading} />
    <Hotspots records={report?.hotspot_records ?? {}} {loading} />
    <LogTail lines={report?.log_tail ?? []} total={report?.ledger_total ?? 0} {loading} />
    <LiveRun {sha} />
  </div>
</div>

<style>
  .col {
    flex: 0 0 320px;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border-on-dark);
    height: 100%;
    background: var(--bg-canvas);
    cursor: pointer;
    transition: flex-basis 0.25s ease-out, background-color 0.25s ease-out;
  }
  .col.current {
    flex: 0 0 520px;
    background: var(--bg-secondary);
    cursor: default;
  }
  .col:not(.current):hover {
    background: var(--bg-dark-hover);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
</style>
