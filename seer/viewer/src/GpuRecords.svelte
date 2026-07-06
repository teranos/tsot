<script lang="ts">
  import type { GpuRecord } from './lib/types'
  import { kindName } from './lib/format'

  let { records, loading }: { records: Record<string, GpuRecord>, loading: boolean } = $props()

  const entries = $derived(
    Object.entries(records).sort((a, b) => b[1].size - a[1].size)
  )
</script>

<section>
  <h3>gpu records <span class="count">{entries.length}</span></h3>
  {#if loading}
    <div class="msg">loading…</div>
  {:else if entries.length === 0}
    <div class="msg">no gpu records</div>
  {:else}
    <ul>
      {#each entries as [id, r]}
        <li>
          <details>
            <summary>
              <span class="id">#{id}</span>
              <span class="kind">{kindName(r.kind)}</span>
              <span class="size">{(r.size / 1024).toFixed(1)} KB</span>
            </summary>
            <pre>{r.backtrace}</pre>
          </details>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  section {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  h3 {
    font-size: var(--font-size-xs);
    color: var(--text-on-dark-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 6px 0;
  }
  .count { color: var(--accent-on-dark); font-weight: 600; margin-left: 4px; }
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  details { background: var(--bg-dark); padding: 4px 8px; border-radius: var(--border-radius); }
  summary {
    cursor: pointer;
    font-size: var(--font-size-sm);
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .id { color: var(--accent-on-dark); }
  .kind { color: var(--text-on-dark-secondary); }
  .size { color: var(--text-on-dark-tertiary); margin-left: auto; }
  pre {
    font-size: var(--font-size-xs);
    margin: 6px 0 0 0;
    color: var(--text-on-dark-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
