<script lang="ts">
  import type { Metric } from './lib/types'

  let { metrics, error }: { metrics: Metric[] | null, error: string } = $props()

  const W = 260
  const H = 32

  function pathOf(vals: number[]): string {
    if (vals.length === 0) return ''
    const max = Math.max(...vals, 1)
    const last = vals.length - 1 || 1
    let s = ''
    for (let i = 0; i < vals.length; i++) {
      const x = (i / last) * W
      const y = H - (vals[i] / max) * (H - 2) - 1
      s += (i === 0 ? 'M' : ' L') + ' ' + x.toFixed(1) + ' ' + y.toFixed(1)
    }
    return s
  }

  const series = $derived.by(() => {
    if (!metrics || metrics.length === 0) return null
    return {
      heap: pathOf(metrics.map(m => m.heap_bytes)),
      gpu_bytes: pathOf(metrics.map(m => m.gpu_bytes)),
      gpu_live: pathOf(metrics.map(m => m.gpu_live)),
      n: metrics.length,
    }
  })
</script>

<div class="spark">
  {#if series}
    <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
      <path d={series.heap} stroke="var(--accent-on-dark)" fill="none" stroke-width="1" />
      <path d={series.gpu_bytes} stroke="var(--color-warning)" fill="none" stroke-width="1" />
      <path d={series.gpu_live} stroke="var(--color-info)" fill="none" stroke-width="1" />
    </svg>
    <div class="legend">
      <span class="s heap">heap</span>
      <span class="s gpu-b">gpu bytes</span>
      <span class="s gpu-l">gpu live</span>
      <span class="n">{series.n} frames</span>
    </div>
  {:else if error}
    <div class="msg">no metrics ({error})</div>
  {:else if metrics === null}
    <div class="msg">loading…</div>
  {:else}
    <div class="msg">no metrics</div>
  {/if}
</div>

<style>
  .spark {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  svg {
    display: block;
    width: 100%;
    height: 32px;
    background: var(--bg-dark);
    border-radius: var(--border-radius);
  }
  .legend {
    display: flex;
    gap: 8px;
    margin-top: 4px;
    font-size: var(--font-size-xs);
    color: var(--text-on-dark-tertiary);
  }
  .s.heap { color: var(--accent-on-dark); }
  .s.gpu-b { color: var(--color-warning); }
  .s.gpu-l { color: var(--color-info); }
  .n { margin-left: auto; }
  .msg {
    padding: 12px 0;
    color: var(--text-on-dark-tertiary);
    font-size: var(--font-size-sm);
    text-align: center;
  }
</style>
