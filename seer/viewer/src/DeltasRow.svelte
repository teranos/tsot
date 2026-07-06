<script lang="ts">
  import type { RunSummary } from './lib/types'
  import { fmtDeltaMB, fmtDeltaCount, deltaClass, fmtWasmMB } from './lib/format'

  let { entry }: { entry: RunSummary } = $props()

  const heapCls = deltaClass(entry.d_heap_mb)
  const gpuLiveCls = deltaClass(entry.d_gpu_live)
  const gpuBytesCls = deltaClass(entry.d_gpu_bytes_mb)
</script>

<div class="deltas">
  <div class="row">
    <span class="label">heap</span>
    <span>{entry.heap_end_mb.toFixed(2)} MB</span>
    <span class="delta {heapCls}">{fmtDeltaMB(entry.d_heap_mb)}</span>
  </div>
  <div class="row">
    <span class="label">gpu live</span>
    <span>{entry.gpu_live_end}</span>
    <span class="delta {gpuLiveCls}">{fmtDeltaCount(entry.d_gpu_live)}</span>
  </div>
  <div class="row">
    <span class="label">gpu bytes</span>
    <span>{entry.gpu_bytes_end_mb.toFixed(2)} MB</span>
    <span class="delta {gpuBytesCls}">{fmtDeltaMB(entry.d_gpu_bytes_mb)}</span>
  </div>
  <div class="row">
    <span class="label">wasm</span>
    <span>{fmtWasmMB(entry.wasm_bytes)}</span>
  </div>
</div>

<style>
  .deltas {
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  .row {
    display: flex;
    justify-content: space-between;
    font-size: var(--font-size-sm);
    gap: 8px;
  }
  .label { color: var(--text-on-dark-tertiary); }
  .delta.up { color: var(--color-error); }
  .delta.down { color: var(--color-success); }
  .delta.flat { color: var(--text-on-dark-tertiary); }
</style>
