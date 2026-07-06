<script lang="ts">
  import type { RunSummary } from './lib/types'
  import { loadHistory, shaFromEntry } from './lib/fetch'
  import { shortSha } from './lib/format'
  import Column from './Column.svelte'

  let history: RunSummary[] = $state([])
  let error = $state('')
  let loading = $state(true)
  let currentSha = $state('')

  // Sort oldest → newest so DOM order matches reading order left→right;
  // the newest lands at the right end of the strip which is what a
  // reader looking at "commit history" expects.
  const sorted = $derived([...history].sort((a, b) => a.when_unix - b.when_unix))

  let stripEl: HTMLDivElement | undefined = $state()

  // Reflect currentSha into ?sha=<short> so the URL is shareable and
  // survives reloads. Uses short sha for readability — prefix match
  // on load handles the full-vs-short question either way.
  function writeUrlSha(sha: string) {
    if (!sha) return
    const url = new URL(window.location.href)
    url.searchParams.set('sha', shortSha(sha))
    window.history.replaceState({}, '', url.toString())
  }

  function focusSha(sha: string) {
    if (!sha || sha === currentSha) return
    currentSha = sha
    writeUrlSha(sha)
    // Wait a paint so the newly-widened column has its final size
    // before scrollIntoView measures.
    requestAnimationFrame(() => {
      if (!stripEl) return
      const col = stripEl.querySelector(`[data-sha="${sha}"]`) as HTMLElement | null
      if (col) col.scrollIntoView({ behavior: 'smooth', inline: 'nearest', block: 'nearest' })
    })
  }

  function currentIndex(): number {
    return sorted.findIndex(e => shaFromEntry(e) === currentSha)
  }

  function step(delta: number) {
    if (sorted.length === 0) return
    const i = currentIndex()
    const next = Math.max(0, Math.min(sorted.length - 1, (i < 0 ? sorted.length - 1 : i) + delta))
    focusSha(shaFromEntry(sorted[next]))
  }

  function scrollFocusedBody(delta: number) {
    if (!stripEl) return
    const body = stripEl.querySelector('.col.current .body') as HTMLElement | null
    if (body) body.scrollBy({ top: delta, behavior: 'smooth' })
  }

  function onKey(e: KeyboardEvent) {
    // Don't hijack keys while user is typing in an input/textarea.
    const t = e.target as HTMLElement | null
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
    if (e.key === 'ArrowRight') { e.preventDefault(); step(+1) }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); step(-1) }
    else if (e.key === 'ArrowDown') { e.preventDefault(); scrollFocusedBody(+240) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); scrollFocusedBody(-240) }
    else if (e.key === 'Home') { e.preventDefault(); if (sorted.length) focusSha(shaFromEntry(sorted[0])) }
    else if (e.key === 'End') { e.preventDefault(); if (sorted.length) focusSha(shaFromEntry(sorted[sorted.length - 1])) }
  }

  async function boot() {
    try {
      history = await loadHistory()
      if (history.length > 0) {
        // Resolve initial focus: ?sha=<prefix> from URL wins if it
        // matches a history entry; otherwise focus the newest.
        const requested = new URLSearchParams(window.location.search).get('sha') || ''
        const match = requested
          ? sorted.find(e => shaFromEntry(e).startsWith(requested))
          : undefined
        const newest = [...history].sort((a, b) => b.when_unix - a.when_unix)[0]
        currentSha = match ? shaFromEntry(match) : shaFromEntry(newest)
        writeUrlSha(currentSha)
      }
    } catch (e: any) {
      error = e?.message || String(e)
    } finally {
      loading = false
    }
  }

  // On first paint after history loads, bring the currentSha column
  // into view. If it's the newest (default), that scrolls all the way
  // right; if it's a deep-linked older sha, that lands mid-strip.
  $effect(() => {
    if (!loading && sorted.length > 0 && stripEl && currentSha) {
      requestAnimationFrame(() => {
        if (!stripEl) return
        const col = stripEl.querySelector(`[data-sha="${currentSha}"]`) as HTMLElement | null
        if (col) col.scrollIntoView({ behavior: 'auto', inline: 'center', block: 'nearest' })
      })
    }
  })

  $effect(() => {
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  boot()
</script>

<div class="seer">
  <header>
    <span class="title">seer</span>
    {#if !loading && !error}
      <span class="stat">{history.length} commits</span>
      {#if currentSha}
        <span class="focus">focus: <code>{shortSha(currentSha)}</code></span>
      {/if}
      <span class="hint">← → step · ↑ ↓ scroll · Home/End ends</span>
    {/if}
  </header>

  {#if loading}
    <div class="msg">loading /history.json…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if history.length === 0}
    <div class="msg">history is empty</div>
  {:else}
    <div class="strip" bind:this={stripEl}>
      {#each sorted as entry (entry.report_url + entry.when_unix)}
        <Column
          {entry}
          current={shaFromEntry(entry) === currentSha}
          onSelect={focusSha}
        />
      {/each}
    </div>
  {/if}
</div>

<style>
  .seer {
    height: 100vh;
    display: flex;
    flex-direction: column;
    color: var(--text-on-dark);
    font-family: var(--font-mono);
    font-size: var(--font-size-md);
    background: var(--bg-canvas);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 6px 12px;
    background: var(--bg-almost-black);
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  .title {
    color: var(--accent-on-dark);
    font-weight: 500;
    font-size: var(--font-size-lg);
  }
  .stat { color: var(--text-on-dark-tertiary); font-size: var(--font-size-sm); }
  .focus { font-size: var(--font-size-sm); color: var(--text-on-dark-secondary); }
  .focus code { color: var(--accent-on-dark); }
  .hint { color: var(--text-on-dark-tertiary); font-size: var(--font-size-xs); margin-left: auto; }
  .msg { color: var(--text-on-dark-tertiary); padding: 24px; text-align: center; }
  .msg.err { color: var(--color-error); }

  .strip {
    flex: 1;
    display: flex;
    overflow-x: auto;
    overflow-y: hidden;
  }
</style>
