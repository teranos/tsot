<script lang="ts">
  import { wasmUrl } from './lib/fetch'

  // Per-column "run this commit's wasm in your browser now" panel.
  // Fetches /perf/<sha>/seer.wasm, wires the same four env.* imports
  // that seer-host provides under wasmtime and that seer/web/index.html
  // used to provide under the browser bootstrap. Streams the wasm's
  // seer_emit output into a scrollable pre so the reader gets ground
  // truth from THIS commit's binary — not an interpretation of it.
  //
  // Only expected to work for shas whose seer.wasm still lives on S3;
  // seer-browser.yml GCs everything beyond the newest 4 (see
  // 'we should always keep 4 recent versions alive'). Older shas get
  // a 404 on click and surface the honest error.

  let { sha }: { sha: string } = $props()

  type State = 'idle' | 'loading' | 'running' | 'done' | 'error'
  let state: State = $state('idle')
  let error = $state('')
  let emits: string[] = $state([])
  let memory: WebAssembly.Memory | null = null

  const MAX_EMITS = 500

  function pushEmit(s: string) {
    if (emits.length >= MAX_EMITS) emits = emits.slice(-MAX_EMITS + 1)
    emits = [...emits, s]
  }

  function decodeString(ptr: number, len: number): string {
    if (!memory) return ''
    const bytes = new Uint8Array(memory.buffer, ptr, len)
    return new TextDecoder('utf-8').decode(bytes)
  }

  async function run() {
    if (state === 'loading' || state === 'running') return
    state = 'loading'
    error = ''
    emits = []
    memory = null
    try {
      const url = wasmUrl(sha)
      const res = await fetch(url)
      if (!res.ok) {
        // Distinguish GC (404) from network/other failure so the
        // reader knows *why*.
        if (res.status === 404) {
          throw new Error(`wasm not on S3 (GC'd — this sha is older than the last 4)`)
        }
        throw new Error(`${url} → HTTP ${res.status}`)
      }
      const bytes = await res.arrayBuffer()
      const imports: WebAssembly.Imports = {
        env: {
          seer_emit: (ptr: number, len: number) => {
            pushEmit(decodeString(ptr, len))
          },
          seer_record_hotspot: (seq: number, size: number, align: number) => {
            pushEmit(`[browser.hotspot] seq=${seq} size=${size} align=${align}`)
          },
          seer_record_gpu_event: (id: number, kind: number, size: number, labelPtr: number, labelLen: number) => {
            const kindName = kind === 1 ? 'buffer' : kind === 2 ? 'texture' : kind === 3 ? 'shader' : `?(${kind})`
            const label = decodeString(labelPtr, labelLen)
            pushEmit(`[browser.gpu] id=${id} kind=${kindName} size=${size} label=${label}`)
          },
          seer_record_gpu_destroyed: (id: number) => {
            pushEmit(`[browser.gpu.destroyed] id=${id}`)
          },
          seer_report_metric: (frame: number, heap: number, live: number, gpuBytes: number) => {
            // Metric emits are frequent — kept out of the on-screen
            // stream but logged for the DevTools console so a real
            // debugger can still pull them out.
            console.log(`[browser.metric] frame=${frame} heap=${heap} gpu_live=${live} gpu_bytes=${gpuBytes}`)
          },
        },
      }
      const { instance } = await WebAssembly.instantiate(bytes, imports)
      memory = instance.exports.memory as WebAssembly.Memory
      state = 'running'
      const runFn = instance.exports.run as (() => void) | undefined
      if (typeof runFn !== 'function') {
        throw new Error(`wasm has no run() export — got: ${Object.keys(instance.exports).join(', ')}`)
      }
      runFn()
      state = 'done'
    } catch (e: any) {
      error = e?.message || String(e)
      state = 'error'
    }
  }
</script>

<section>
  <h3>run this wasm in browser</h3>
  <div class="controls">
    <button onclick={run} disabled={state === 'loading' || state === 'running'}>
      {#if state === 'idle'}▶ run{:else if state === 'loading'}fetching…{:else if state === 'running'}running…{:else if state === 'done'}▶ re-run{:else}▶ retry{/if}
    </button>
    {#if state === 'done'}
      <span class="meta">{emits.length} emit{emits.length === 1 ? '' : 's'}</span>
    {/if}
  </div>
  {#if state === 'error'}
    <div class="err">{error}</div>
  {/if}
  {#if emits.length > 0}
    <pre>{emits.join('\n')}</pre>
  {/if}
</section>

<style>
  section {
    padding: 8px 12px;
    border-top: 1px solid var(--border-on-dark);
    flex: 0 0 auto;
  }
  h3 {
    font-size: var(--font-size-xs);
    color: var(--text-on-dark-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 6px 0;
  }
  .controls {
    display: flex;
    gap: 10px;
    align-items: baseline;
    margin-bottom: 6px;
  }
  button {
    background: var(--bg-dark-light);
    color: var(--accent-on-dark);
    border: 1px solid var(--border-on-dark);
    padding: 4px 10px;
    font: inherit;
    font-size: var(--font-size-sm);
    cursor: pointer;
    border-radius: var(--border-radius);
  }
  button:hover:not(:disabled) { background: var(--bg-dark-hover); }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  .meta { color: var(--text-on-dark-tertiary); font-size: var(--font-size-xs); }
  .err {
    color: var(--color-error);
    font-size: var(--font-size-sm);
    padding: 6px 8px;
    background: var(--bg-dark);
    border-left: 2px solid var(--color-error);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    word-break: break-word;
  }
  pre {
    background: var(--bg-dark);
    color: var(--text-on-dark-secondary);
    padding: 8px 10px;
    border-radius: var(--border-radius);
    font-size: var(--font-size-xs);
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 320px;
    overflow-y: auto;
  }
</style>
