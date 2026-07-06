<script lang="ts">
  let { lines, total, loading }: { lines: string[], total: number, loading: boolean } = $props()
</script>

<section>
  <details>
    <summary>
      <span class="label">log tail</span>
      {#if !loading}
        <span class="count">{lines.length} of {total}</span>
      {/if}
    </summary>
    {#if loading}
      <div class="msg">loading…</div>
    {:else if lines.length === 0}
      <div class="msg">no signal lines in ledger</div>
    {:else}
      <pre>{lines.join('\n')}</pre>
    {/if}
  </details>
</section>

<style>
  section {
    padding: 8px 12px;
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
  .count { color: var(--text-on-dark-tertiary); font-weight: normal; margin-left: 6px; text-transform: none; letter-spacing: 0; }
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); margin-top: 6px; }
  pre {
    background: var(--bg-dark);
    color: var(--text-on-dark-secondary);
    padding: 8px 10px;
    border-radius: var(--border-radius);
    font-size: var(--font-size-xs);
    margin: 6px 0 0 0;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 240px;
    overflow-y: auto;
  }
</style>
