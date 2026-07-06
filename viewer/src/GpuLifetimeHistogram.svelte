<script lang="ts">
  import type { GpuRecord } from './lib/types'

  let { records, loading }: { records: Record<string, GpuRecord>, loading: boolean } = $props()

  // Log2-ish buckets: lifetimes in ledger-index units. Same bucketing
  // as `perf top` / rustc self-profile — order-of-magnitude bins so
  // the shape of the distribution is legible without picking a
  // domain-specific bin size.
  const BUCKETS: { label: string, min: number, max: number }[] = [
    { label: '0-9',      min: 0,     max: 10     },
    { label: '10-99',    min: 10,    max: 100    },
    { label: '100-999',  min: 100,   max: 1000   },
    { label: '1k-9k',    min: 1000,  max: 10000  },
    { label: '10k+',     min: 10000, max: Infinity },
  ]

  interface HistogramRow {
    label: string
    count: number
  }

  const stats = $derived.by(() => {
    const values = Object.values(records)
    const finished = values.filter(r => r.destroyed_at_seq !== null)
    const live = values.length - finished.length
    const buckets: HistogramRow[] = BUCKETS.map(b => ({ label: b.label, count: 0 }))
    for (const r of finished) {
      const life = (r.destroyed_at_seq as number) - r.created_at_seq
      for (let i = 0; i < BUCKETS.length; i++) {
        if (life >= BUCKETS[i].min && life < BUCKETS[i].max) {
          buckets[i].count += 1
          break
        }
      }
    }
    const max = Math.max(1, ...buckets.map(b => b.count), live)
    return { buckets, live, max, total: values.length, finished: finished.length }
  })

  // Inline SVG bar chart. One bar per bucket + one for still-live.
  // Width proportional to count / max; label + count sit inline.
  const W = 260
  const BAR_H = 12
  const BAR_GAP = 3
  const liveY = $derived(BUCKETS.length * (BAR_H + BAR_GAP))
  const liveW = $derived((stats.live / stats.max) * W)
  const svgH = $derived((BUCKETS.length + 1) * (BAR_H + BAR_GAP))
</script>

<section>
  <details>
    <summary>
      <span class="label">gpu lifetime</span>
      {#if !loading}
        <span class="count">{stats.finished} destroyed · {stats.live} live</span>
      {/if}
    </summary>
    {#if loading}
      <div class="msg">loading…</div>
    {:else if stats.total === 0}
      <div class="msg">no gpu records</div>
    {:else}
      <svg viewBox={`0 0 ${W} ${svgH}`} preserveAspectRatio="none">
        {#each stats.buckets as row, i}
          {@const y = i * (BAR_H + BAR_GAP)}
          {@const w = (row.count / stats.max) * W}
          <rect x="0" y={y} width={w} height={BAR_H} fill="var(--accent-on-dark)" />
          <text x="6" y={y + BAR_H - 3} fill="var(--bg-almost-black)" font-size="9" font-family="var(--font-mono)">
            {row.label} · {row.count}
          </text>
        {/each}
        <rect x="0" y={liveY} width={liveW} height={BAR_H} fill="var(--color-warning)" />
        <text x="6" y={liveY + BAR_H - 3} fill="var(--bg-almost-black)" font-size="9" font-family="var(--font-mono)">
          still live · {stats.live}
        </text>
      </svg>
    {/if}
  </details>
</section>

<style>
  section {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  details > summary {
    cursor: pointer;
    font-size: var(--font-size-xs);
    color: var(--text-on-dark-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    list-style: revert;
    display: block;
  }
  .label { font-weight: 500; }
  .count {
    color: var(--text-on-dark-tertiary);
    font-weight: normal;
    margin-left: 6px;
    text-transform: none;
    letter-spacing: 0;
  }
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); margin-top: 6px; }
  svg {
    display: block;
    width: 100%;
    height: auto;
    margin-top: 6px;
    background: var(--bg-dark);
    border-radius: var(--border-radius);
  }
</style>
