<script lang="ts">
  import type { GpuRecord } from './lib/types'
  import { kindName } from './lib/format'

  let { records, loading }: { records: Record<string, GpuRecord>, loading: boolean } = $props()

  // Split by kind so each list is one category (buffer / texture /
  // shader). Sparklines above already plot the aggregate count over
  // time; this section is per-resource detail, so it shouldn't
  // dominate the column by default.
  const entries = $derived(
    Object.entries(records).sort((a, b) => b[1].size - a[1].size)
  )
  const buffers = $derived(entries.filter(([, r]) => r.kind === 1))
  const textures = $derived(entries.filter(([, r]) => r.kind === 2))
  const shaders = $derived(entries.filter(([, r]) => r.kind === 3))

  const VISIBLE_HEAD = 5

  // Independent expand state per kind — clicking "show all" on
  // buffers doesn't expand textures.
  let showAll = $state<Record<number, boolean>>({ 1: false, 2: false, 3: false })

  function toggleAll(kind: number) {
    showAll[kind] = !showAll[kind]
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
                      <span class="kind">{kindName(r.kind)}</span>
                      <span class="size">{(r.size / 1024).toFixed(1)} KB</span>
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
  .label {
    font-weight: 500;
  }
  .count {
    color: var(--accent-on-dark);
    font-weight: 600;
    margin-left: 6px;
    text-transform: none;
    letter-spacing: 0;
  }
  .split {
    color: var(--text-on-dark-tertiary);
    font-weight: normal;
    margin-left: 8px;
    text-transform: none;
    letter-spacing: 0;
    font-size: 10px;
  }
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); margin-top: 6px; }
  .group {
    margin-top: 8px;
  }
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
  .kind { color: var(--text-on-dark-secondary); }
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
