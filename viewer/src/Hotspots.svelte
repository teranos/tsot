<script lang="ts">
  import type { HotspotRecord } from './lib/types'

  let { records, loading }: { records: Record<string, HotspotRecord>, loading: boolean } = $props()

  const entries = $derived(
    Object.entries(records).sort((a, b) => b[1].size - a[1].size)
  )

  const VISIBLE_HEAD = 5
  let showAll = $state(false)
</script>

<section>
  <details>
    <summary>
      <span class="label">hotspots</span>
      <span class="count">{entries.length}</span>
    </summary>
    {#if loading}
      <div class="msg">loading…</div>
    {:else if entries.length === 0}
      <div class="msg">no heap hotspots ≥ threshold</div>
    {:else}
      <ul>
        {#each showAll ? entries : entries.slice(0, VISIBLE_HEAD) as [seq, r] (seq)}
          <li>
            <details>
              <summary>
                <span class="id">seq {seq}</span>
                <span class="size">{(r.size / 1024).toFixed(1)} KB</span>
                <span class="align">align {r.align}</span>
              </summary>
              <pre>{r.backtrace}</pre>
            </details>
          </li>
        {/each}
      </ul>
      {#if entries.length > VISIBLE_HEAD}
        <button class="more" onclick={() => (showAll = !showAll)}>
          {showAll ? `show first ${VISIBLE_HEAD}` : `show all ${entries.length}`}
        </button>
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
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); margin-top: 6px; }
  ul { list-style: none; margin: 6px 0 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
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
  .size { color: var(--text-on-dark-secondary); margin-left: auto; }
  .align { color: var(--text-on-dark-tertiary); }
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
