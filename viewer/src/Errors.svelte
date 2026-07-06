<script lang="ts">
  let { errors, loading, error }: { errors: string[], loading: boolean, error: string } = $props()
</script>

<section>
  <h3>
    errors
    {#if !loading && !error}
      <span class="count" class:zero={errors.length === 0} class:some={errors.length > 0}>
        {errors.length}
      </span>
    {/if}
  </h3>
  {#if loading}
    <div class="msg">loading…</div>
  {:else if error}
    <div class="msg">no report ({error})</div>
  {:else if errors.length === 0}
    <div class="msg">no sacred errors captured</div>
  {:else}
    <ul>
      {#each errors as e}
        <li>{e}</li>
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
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
  .count.zero { color: var(--text-on-dark-tertiary); }
  .count.some { color: var(--color-error); font-weight: 600; }
  .msg { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  li {
    font-size: var(--font-size-sm);
    color: var(--color-error);
    padding: 4px 6px;
    background: var(--bg-dark);
    border-left: 2px solid var(--color-error);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    word-break: break-word;
  }
</style>
