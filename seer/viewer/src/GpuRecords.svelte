<script lang="ts">
  import type { GpuRecord } from './lib/types'
  import { kindName } from './lib/format'

  let { records, loading }: { records: Record<string, GpuRecord>, loading: boolean } = $props()

  const entries = $derived(
    Object.entries(records).sort((a, b) => b[1].size - a[1].size)
  )
  const buffers = $derived(entries.filter(([, r]) => r.kind === 1))
  const textures = $derived(entries.filter(([, r]) => r.kind === 2))
  const shaders = $derived(entries.filter(([, r]) => r.kind === 3))

  // Aggregation by label — Task 6. Same label may be created many
  // times (churned resources): the rollup lets the reader spot which
  // label dominates the record count and byte total without scrolling
  // the per-instance list. Grouping ignores kind; a label is a label.
  interface LabelRow {
    label: string
    count: number
    total_bytes: number
    live_count: number
    kinds: Set<number>
  }
  const byLabel = $derived.by(() => {
    const map = new Map<string, LabelRow>()
    for (const [, r] of entries) {
      const key = r.label || '<unlabelled>'
      const row = map.get(key) ?? {
        label: key,
        count: 0,
        total_bytes: 0,
        live_count: 0,
        kinds: new Set<number>(),
      }
      row.count += 1
      row.total_bytes += r.size
      if (r.destroyed_at_seq === null) row.live_count += 1
      row.kinds.add(r.kind)
      map.set(key, row)
    }
    return [...map.values()].sort((a, b) => b.total_bytes - a.total_bytes)
  })

  const VISIBLE_HEAD = 5
  let showAll = $state<Record<number, boolean>>({ 1: false, 2: false, 3: false })
  let mode: 'by-label' | 'per-instance' = $state('by-label')

  function toggleAll(kind: number) {
    showAll[kind] = !showAll[kind]
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1_048_576) return `${(n / 1024).toFixed(1)} KB`
    return `${(n / 1_048_576).toFixed(2)} MB`
  }

  function kindsChip(kinds: Set<number>): string {
    return [...kinds].map(kindName).join(', ')
  }
</script>

<section>
  <details>
    <summary>
      <span class="label">gpu records</span>
      <span class="count">{entries.length}</span>
      <span class="split">
        {#if buffers.length > 0}buffers {buffers.length}{/if}
        {#if textures.length > 0} · textures {textures.length}{/if}
        {#if shaders.length > 0} · shaders {shaders.length}{/if}
      </span>
    </summary>

    {#if loading}
      <div class="msg">loading…</div>
    {:else if entries.length === 0}
      <div class="msg">no gpu records</div>
    {:else}
      <div class="mode">
        <button class:on={mode === 'by-label'} onclick={() => (mode = 'by-label')}>by label</button>
        <button class:on={mode === 'per-instance'} onclick={() => (mode = 'per-instance')}>per instance</button>
      </div>

      {#if mode === 'by-label'}
        <table>
          <thead>
            <tr>
              <th>label</th>
              <th>count</th>
              <th>total</th>
              <th>live</th>
              <th>kinds</th>
            </tr>
          </thead>
          <tbody>
            {#each byLabel as row (row.label)}
              <tr>
                <td class="lbl">{row.label}</td>
                <td class="num">{row.count}</td>
                <td class="num">{fmtBytes(row.total_bytes)}</td>
                <td class="num">{row.live_count}</td>
                <td class="kinds">{kindsChip(row.kinds)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        {#each [{ kind: 1, list: buffers }, { kind: 2, list: textures }, { kind: 3, list: shaders }] as group (group.kind)}
          {#if group.list.length > 0}
            <div class="group">
              <div class="group-hd">{kindName(group.kind)}s <span class="group-count">{group.list.length}</span></div>
              <ul>
                {#each showAll[group.kind] ? group.list : group.list.slice(0, VISIBLE_HEAD) as [id, r] (id)}
                  <li>
                    <details>
                      <summary>
                        <span class="id">#{id}</span>
                        <span class="lbl inline">{r.label || '<unlabelled>'}</span>
                        <span class="size">{fmtBytes(r.size)}</span>
                      </summary>
                      <pre>{r.backtrace}</pre>
                    </details>
                  </li>
                {/each}
              </ul>
              {#if group.list.length > VISIBLE_HEAD}
                <button class="more" onclick={() => toggleAll(group.kind)}>
                  {showAll[group.kind]
                    ? `show first ${VISIBLE_HEAD}`
                    : `show all ${group.list.length}`}
                </button>
              {/if}
            </div>
          {/if}
        {/each}
      {/if}
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
  .count { color: var(--accent-on-dark); font-weight: 600; margin-left: 6px; text-transform: none; letter-spacing: 0; }
  .split { color: var(--text-on-dark-tertiary); font-weight: normal; margin-left: 8px; text-transform: none; letter-spacing: 0; font-size: 10px; }
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); margin-top: 6px; }

  .mode {
    display: flex;
    gap: 4px;
    margin: 6px 0;
  }
  .mode button {
    background: transparent;
    border: 1px solid var(--border-on-dark);
    color: var(--text-on-dark-tertiary);
    padding: 2px 8px;
    font: inherit;
    font-size: 10px;
    cursor: pointer;
    border-radius: var(--border-radius);
  }
  .mode button.on {
    background: var(--bg-dark-light);
    color: var(--accent-on-dark);
    border-color: var(--accent-on-dark);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--font-size-sm);
    margin-top: 4px;
  }
  th {
    text-align: left;
    color: var(--text-on-dark-tertiary);
    font-size: 10px;
    font-weight: normal;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 3px 6px;
    border-bottom: 1px solid var(--border-on-dark);
  }
  td { padding: 3px 6px; }
  td.num { text-align: right; font-variant-numeric: tabular-nums; }
  td.lbl { color: var(--text-on-dark); word-break: break-word; }
  td.kinds { color: var(--text-on-dark-tertiary); font-size: 10px; }

  .group { margin-top: 8px; }
  .group-hd {
    font-size: 10px;
    color: var(--text-on-dark-secondary);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 3px;
  }
  .group-count {
    color: var(--accent-on-dark);
    font-weight: 600;
    margin-left: 4px;
    letter-spacing: 0;
    text-transform: none;
  }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  li details { background: var(--bg-dark); padding: 4px 8px; border-radius: var(--border-radius); }
  li summary {
    cursor: pointer;
    font-size: var(--font-size-sm);
    display: flex;
    gap: 10px;
    align-items: baseline;
    color: var(--text-on-dark);
    text-transform: none;
    letter-spacing: 0;
  }
  .id { color: var(--accent-on-dark); }
  .lbl.inline { color: var(--text-on-dark-secondary); font-size: 10px; }
  .size { color: var(--text-on-dark-tertiary); margin-left: auto; }
  pre {
    font-size: var(--font-size-xs);
    margin: 6px 0 0 0;
    color: var(--text-on-dark-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .more {
    display: block;
    width: 100%;
    margin-top: 4px;
    background: transparent;
    border: 1px dashed var(--border-on-dark);
    color: var(--accent-on-dark);
    padding: 3px 8px;
    font: inherit;
    font-size: var(--font-size-xs);
    cursor: pointer;
    border-radius: var(--border-radius);
  }
  .more:hover { background: var(--bg-dark); }
</style>
