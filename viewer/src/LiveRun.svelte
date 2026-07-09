<script lang="ts">
  import { wasmUrl } from './lib/fetch'

  // Per-column "run this commit's wasm in your browser now" panel.
  // Fetches /perf/<sha>/game.wasm, wires the env.* imports that
  // seer-host provides under wasmtime. Streams the wasm's seer_emit
  // output into a scrollable pre — ground truth from THIS commit's
  // binary. Older shas whose game.wasm has been GC'd surface a 404.

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
      // Encapsulated WebGPU init: the shim owns the async chain
      // (navigator.gpu → requestAdapter → requestDevice). Wasm reads
      // gpuStatus to observe progress. 0=pending, 1=ready, 2=unavailable.
      let gpuStatus = 0
      let gpuDevice: any = null
      const gpuBuffers = new Map<number, any>()
      const gpuShaders = new Map<number, any>()
      const gpuBindGroupLayouts = new Map<number, any>()
      const gpuBindGroups = new Map<number, any>()
      const gpuPipelineLayouts = new Map<number, any>()
      const gpuRenderPipelines = new Map<number, any>()
      let nextGpuHandle = 1
      const LIVE_COLOR_FORMATS = ['rgba8unorm', 'bgra8unorm'] as const
      const LIVE_DEPTH_FORMATS = ['depth32float', 'depth24plus'] as const
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
          game_gpu_init: (powerPref: number) => {
            const nav = navigator as any
            if (!nav.gpu) {
              gpuStatus = 2
              pushEmit('[browser.gpu_init] navigator.gpu missing — unavailable')
              return
            }
            const powerPreference = powerPref === 1 ? 'high-performance' : 'low-power'
            nav.gpu.requestAdapter({ powerPreference })
              .then((adapter: any) => {
                if (!adapter) throw new Error('no adapter')
                return adapter.requestDevice()
              })
              .then((device: any) => {
                gpuDevice = device
                gpuStatus = 1
                pushEmit('[browser.gpu_init] device ready')
              })
              .catch((e: any) => {
                gpuStatus = 2
                pushEmit(`[browser.gpu_init] failed: ${e?.message || e}`)
              })
          },
          game_gpu_status: (): number => gpuStatus,
          game_gpu_buffer_create: (size: number, usage: number, labelPtr: number, labelLen: number): number => {
            if (!gpuDevice) return 0
            try {
              const label = decodeString(labelPtr, labelLen)
              const buf = gpuDevice.createBuffer({ size, usage, label })
              const h = nextGpuHandle++
              gpuBuffers.set(h, buf)
              pushEmit(`[browser.gpu_buffer_create] handle=${h} size=${size} usage=${usage.toString(16)} label=${label}`)
              return h
            } catch (e: any) {
              pushEmit(`[browser.gpu_buffer_create] failed: ${e?.message || e}`)
              return 0
            }
          },
          game_gpu_buffer_write: (handle: number, dataPtr: number, dataLen: number) => {
            if (!gpuDevice || !memory) return
            const buf = gpuBuffers.get(handle)
            if (!buf) return
            const view = new Uint8Array(memory.buffer, dataPtr, dataLen)
            gpuDevice.queue.writeBuffer(buf, 0, view)
          },
          game_gpu_buffer_destroy: (handle: number) => {
            const buf = gpuBuffers.get(handle)
            if (!buf) return
            buf.destroy()
            gpuBuffers.delete(handle)
            pushEmit(`[browser.gpu_buffer_destroy] handle=${handle}`)
          },
          game_gpu_shader_module_create: (srcPtr: number, srcLen: number, labelPtr: number, labelLen: number): number => {
            if (!gpuDevice) return 0
            try {
              const code = decodeString(srcPtr, srcLen)
              const label = decodeString(labelPtr, labelLen)
              const mod = gpuDevice.createShaderModule({ code, label })
              const h = nextGpuHandle++
              gpuShaders.set(h, mod)
              pushEmit(`[browser.gpu_shader] handle=${h} label=${label} src_len=${srcLen}`)
              return h
            } catch (e: any) {
              pushEmit(`[browser.gpu_shader] failed: ${e?.message || e}`)
              return 0
            }
          },
          game_gpu_bind_group_layout_create_uniform: (labelPtr: number, labelLen: number): number => {
            if (!gpuDevice) return 0
            try {
              const label = decodeString(labelPtr, labelLen)
              const bgl = gpuDevice.createBindGroupLayout({
                label,
                entries: [{ binding: 0, visibility: 1 /* GPUShaderStage.VERTEX */, buffer: { type: 'uniform' } }],
              })
              const h = nextGpuHandle++
              gpuBindGroupLayouts.set(h, bgl)
              return h
            } catch (e: any) {
              pushEmit(`[browser.gpu_bgl] failed: ${e?.message || e}`)
              return 0
            }
          },
          game_gpu_bind_group_create: (layoutH: number, bufferH: number, labelPtr: number, labelLen: number): number => {
            if (!gpuDevice) return 0
            const layout = gpuBindGroupLayouts.get(layoutH)
            const buffer = gpuBuffers.get(bufferH)
            if (!layout || !buffer) return 0
            try {
              const label = decodeString(labelPtr, labelLen)
              const bg = gpuDevice.createBindGroup({
                label,
                layout,
                entries: [{ binding: 0, resource: { buffer } }],
              })
              const h = nextGpuHandle++
              gpuBindGroups.set(h, bg)
              return h
            } catch (e: any) {
              pushEmit(`[browser.gpu_bg] failed: ${e?.message || e}`)
              return 0
            }
          },
          game_gpu_pipeline_layout_create: (bgLayoutH: number, labelPtr: number, labelLen: number): number => {
            if (!gpuDevice) return 0
            const bgl = gpuBindGroupLayouts.get(bgLayoutH)
            if (!bgl) return 0
            try {
              const label = decodeString(labelPtr, labelLen)
              const pl = gpuDevice.createPipelineLayout({ label, bindGroupLayouts: [bgl] })
              const h = nextGpuHandle++
              gpuPipelineLayouts.set(h, pl)
              return h
            } catch (e: any) {
              pushEmit(`[browser.gpu_pl] failed: ${e?.message || e}`)
              return 0
            }
          },
          game_gpu_render_target_configure: (_canvasIdPtr: number, _canvasIdLen: number, _colorFormat: number, _depthFormat: number): number => {
            // LiveRun is diagnostic replay — no canvas surface. Return
            // 0 so the Rust demo skips the render_frame call cleanly.
            return 0
          },
          game_gpu_render_frame: (
            _targetH: number, _pipelineH: number, _bindGroupH: number,
            _vertexBufH: number, _instanceBufH: number,
            _vertexCount: number, _instanceCount: number,
            _clearR: number, _clearG: number, _clearB: number,
          ): number => 1,
          game_input_state: (): number => 0,
          game_show_exclamation: (_x: number, _y: number) => {},
          game_identity_load: (_outPtr: number): number => 0,
          game_identity_save: (_bytesPtr: number, _bytesLen: number) => {},
          game_random_bytes: (_outPtr: number, _outLen: number) => {},
          game_peers_pending: (): number => 0,
          game_peers_recv: (_outPtr: number, _outLen: number): number => 0,
          game_self_publish: (_bytesPtr: number, _bytesLen: number) => {},
          game_now_ms: (): number => Date.now(),
          game_audio_load: (_pathPtr: number, _pathLen: number): number => 0,
          game_audio_play: (_h: number, _vol: number, _loopFlag: number) => {},
          game_audio_stop: (_h: number) => {},
          game_audio_play_samples: (_ptr: number, _count: number, _rate: number) => {},
          game_touch_state: (_outPtr: number, _outMax: number): number => 0,
          game_viewport_size: (_outPtr: number) => {},
          game_gpu_render_pipeline_create_ui: (
            _pipelineLayoutH: number, _shaderH: number, _instanceStride: number,
            _colorFormat: number, _labelPtr: number, _labelLen: number,
          ): number => 1,
          game_gpu_render_ui_overlay: (
            _targetH: number, _pipelineH: number, _bindGroupH: number,
            _instanceBufH: number, _instanceCount: number,
          ): number => 0,
          game_gpu_render_pipeline_create_cube: (
            pipelineLayoutH: number, shaderH: number, vertexStride: number, instanceStride: number,
            colorFormat: number, depthFormat: number, labelPtr: number, labelLen: number,
          ): number => {
            if (!gpuDevice) return 0
            const pl = gpuPipelineLayouts.get(pipelineLayoutH)
            const shader = gpuShaders.get(shaderH)
            if (!pl || !shader) return 0
            const colorFmt = LIVE_COLOR_FORMATS[colorFormat]
            const depthFmt = LIVE_DEPTH_FORMATS[depthFormat]
            if (!colorFmt || !depthFmt) return 0
            try {
              const label = decodeString(labelPtr, labelLen)
              const pipeline = gpuDevice.createRenderPipeline({
                label,
                layout: pl,
                vertex: {
                  module: shader,
                  entryPoint: 'vs',
                  buffers: [
                    { arrayStride: vertexStride, stepMode: 'vertex', attributes: [
                      { shaderLocation: 0, offset: 0, format: 'float32x3' },
                      { shaderLocation: 1, offset: 12, format: 'float32x3' },
                    ] },
                    { arrayStride: instanceStride, stepMode: 'instance', attributes: [
                      { shaderLocation: 2, offset: 0, format: 'float32x3' },
                      { shaderLocation: 3, offset: 12, format: 'float32x3' },
                      { shaderLocation: 4, offset: 24, format: 'float32x3' },
                    ] },
                  ],
                },
                primitive: { topology: 'triangle-list', frontFace: 'ccw', cullMode: 'back' },
                depthStencil: { format: depthFmt, depthWriteEnabled: true, depthCompare: 'less' },
                fragment: { module: shader, entryPoint: 'fs', targets: [{ format: colorFmt }] },
              })
              const h = nextGpuHandle++
              gpuRenderPipelines.set(h, pipeline)
              pushEmit(`[browser.gpu_pipeline] handle=${h} label=${label}`)
              return h
            } catch (e: any) {
              pushEmit(`[browser.gpu_pipeline] failed: ${e?.message || e}`)
              return 0
            }
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
