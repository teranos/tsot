<script lang="ts">
  import { DATA_BASE } from './lib/fetch'

  // Task 9 — multi-frame render + frame-diff. Fetches
  // /perf/<sha>/frame-0..3.png for this commit (4 samples across
  // the tick loop). Click a frame to select A; click a second to
  // select B and switch to a canvas-based |A - B| pixel diff.
  // Click the diff to cycle back to normal thumbs. Older commits
  // that didn't run multi-frame in CI show only frame.png as a
  // single thumbnail — the loader silently skips missing images.

  let { sha }: { sha: string } = $props()

  const N = 4

  let statuses: ('loading' | 'ok' | 'missing')[] = $state(Array(N).fill('loading'))
  let hasSingle = $state(false)   // fallback for pre-Task-9 shas
  let selA: number | null = $state(null)
  let selB: number | null = $state(null)
  let diffCanvas: HTMLCanvasElement | null = $state(null)

  function frameUrl(i: number): string {
    return `${DATA_BASE}/${sha}/frame-${i}.png`
  }
  function singleFrameUrl(): string {
    return `${DATA_BASE}/${sha}/frame.png`
  }

  // HEAD-probe on mount to figure out which frames actually exist.
  // Prevents broken-image icons for pre-Task-9 commits and keeps
  // the layout size stable.
  async function probe() {
    const results = await Promise.all(
      Array.from({ length: N }, (_, i) =>
        fetch(frameUrl(i), { method: 'HEAD' }).then(r => r.ok).catch(() => false)
      )
    )
    statuses = results.map(ok => (ok ? 'ok' : 'missing'))
    if (results.every(ok => !ok)) {
      // No multi-frame — try single frame as a fallback.
      hasSingle = await fetch(singleFrameUrl(), { method: 'HEAD' })
        .then(r => r.ok)
        .catch(() => false)
    }
  }

  $effect(() => {
    probe()
  })

  function onThumbClick(i: number) {
    if (selA === null) {
      selA = i
      return
    }
    if (selA === i) {
      selA = null
      selB = null
      return
    }
    selB = i
    // Kick off diff render on next paint.
    requestAnimationFrame(() => renderDiff(selA!, selB!))
  }

  function clearDiff() {
    selA = null
    selB = null
  }

  async function renderDiff(aIdx: number, bIdx: number) {
    if (!diffCanvas) return
    const [imgA, imgB] = await Promise.all([loadImage(frameUrl(aIdx)), loadImage(frameUrl(bIdx))])
    const w = imgA.naturalWidth
    const h = imgA.naturalHeight
    diffCanvas.width = w
    diffCanvas.height = h
    const ctx = diffCanvas.getContext('2d')
    if (!ctx) return
    ctx.drawImage(imgA, 0, 0)
    const a = ctx.getImageData(0, 0, w, h)
    ctx.drawImage(imgB, 0, 0)
    const b = ctx.getImageData(0, 0, w, h)
    const out = ctx.createImageData(w, h)
    for (let i = 0; i < a.data.length; i += 4) {
      out.data[i] = Math.abs(a.data[i] - b.data[i])
      out.data[i + 1] = Math.abs(a.data[i + 1] - b.data[i + 1])
      out.data[i + 2] = Math.abs(a.data[i + 2] - b.data[i + 2])
      out.data[i + 3] = 255
    }
    ctx.putImageData(out, 0, 0)
  }

  function loadImage(url: string): Promise<HTMLImageElement> {
    return new Promise((resolve, reject) => {
      const img = new Image()
      img.crossOrigin = 'anonymous'
      img.onload = () => resolve(img)
      img.onerror = reject
      img.src = url
    })
  }
</script>

<section>
  {#if hasSingle && statuses.every(s => s === 'missing')}
    <img class="single" src={singleFrameUrl()} alt="frame for {sha}" />
  {:else if statuses.some(s => s === 'ok')}
    {#if selA !== null && selB !== null}
      <div class="diff">
        <canvas bind:this={diffCanvas}></canvas>
        <div class="hint">
          diff <code>{selA}</code> ↔ <code>{selB}</code>
          <button onclick={clearDiff}>clear</button>
        </div>
      </div>
    {:else}
      <div class="strip">
        {#each statuses as st, i}
          {#if st === 'ok'}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
            <img
              src={frameUrl(i)}
              alt="frame {i}"
              class:selected={selA === i}
              onclick={() => onThumbClick(i)}
            />
          {:else if st === 'loading'}
            <div class="ph">…</div>
          {:else}
            <div class="ph gone">gc'd</div>
          {/if}
        {/each}
      </div>
      <div class="hint">
        {#if selA === null}
          click a frame to start a diff
        {:else}
          click another frame to diff against <code>{selA}</code>
          <button onclick={clearDiff}>cancel</button>
        {/if}
      </div>
    {/if}
  {:else}
    <div class="ph gone">no frames rendered</div>
  {/if}
</section>

<style>
  section {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  .strip {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 3px;
  }
  .strip img, .strip .ph {
    width: 100%;
    aspect-ratio: 1;
    display: block;
    border-radius: var(--border-radius);
    background: var(--bg-dark);
    cursor: pointer;
    border: 1px solid transparent;
  }
  .strip img.selected {
    border-color: var(--accent-on-dark);
  }
  .strip img:hover {
    border-color: var(--text-on-dark-secondary);
  }
  .ph {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-on-dark-tertiary);
    font-size: 10px;
  }
  .ph.gone {
    color: var(--text-on-dark-tertiary);
    font-style: italic;
  }
  .single {
    display: block;
    width: 100%;
    height: auto;
    border-radius: var(--border-radius);
  }
  .diff {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .diff canvas {
    display: block;
    width: 100%;
    height: auto;
    background: var(--bg-dark);
    border-radius: var(--border-radius);
  }
  .hint {
    margin-top: 4px;
    font-size: var(--font-size-xs);
    color: var(--text-on-dark-tertiary);
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .hint code {
    color: var(--accent-on-dark);
    background: var(--bg-dark);
    padding: 0 4px;
    border-radius: var(--border-radius);
  }
  .hint button {
    background: transparent;
    border: 1px solid var(--border-on-dark);
    color: var(--accent-on-dark);
    padding: 1px 6px;
    font: inherit;
    font-size: 10px;
    cursor: pointer;
    border-radius: var(--border-radius);
  }
</style>
