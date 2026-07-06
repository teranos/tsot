<script lang="ts">
  import { frameUrl } from './lib/fetch'

  let { sha }: { sha: string } = $props()
  let ok = $state(true)
</script>

<div class="frame">
  {#if ok}
    <img
      src={frameUrl(sha)}
      alt={`scene minimap for ${sha}`}
      onerror={() => { ok = false }}
    />
  {:else}
    <div class="missing">no frame.png</div>
  {/if}
</div>

<style>
  .frame {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: var(--border-radius);
  }
  .missing {
    padding: 24px;
    text-align: center;
    color: var(--text-on-dark-tertiary);
    font-size: var(--font-size-sm);
    background: var(--bg-dark);
    border-radius: var(--border-radius);
  }
</style>
