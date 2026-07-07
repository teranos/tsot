// game.sbvh.nl bootstrap. Fetches /game.wasm with byte-progress
// streaming, wires seven env.* imports, drives requestAnimationFrame.
// Loading screen matches laye/bevy-starter.
//
// Encapsulated GPU init: JS runs navigator.gpu.requestAdapter →
// requestDevice BEFORE wasm.init(). Wasm reads game_gpu_status —
// Ready or Unavailable, never Pending.

let memory: WebAssembly.Memory | null = null

function decodeString(ptr: number, len: number): string {
  if (!memory) return ''
  const bytes = new Uint8Array(memory.buffer, ptr, len)
  return new TextDecoder('utf-8').decode(bytes)
}

const loadingText = document.getElementById('game-loading-text')
const loadingBar = document.querySelector('#game-loading progress') as HTMLProgressElement | null

function fmtMB(bytes: number): string {
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}

function updateProgress(loaded: number, total: number) {
  if (!loadingText || !loadingBar) return
  if (total > 0) {
    const pct = Math.floor((loaded / total) * 100)
    loadingBar.value = loaded
    loadingBar.max = total
    loadingText.textContent = `loading game · ${pct}% · ${fmtMB(loaded)}/${fmtMB(total)}`
  } else {
    loadingText.textContent = `loading game · ${fmtMB(loaded)}`
  }
}

async function streamWasmBytes(url: string): Promise<Uint8Array> {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`fetch ${url} → HTTP ${response.status}`)
  const total = parseInt(response.headers.get('content-length') ?? '0', 10)
  const body = response.body
  if (!body) throw new Error(`fetch ${url} → response.body missing`)
  const reader = body.getReader()
  const chunks: Uint8Array[] = []
  let loaded = 0
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    if (value) {
      chunks.push(value)
      loaded += value.length
      updateProgress(loaded, total)
    }
  }
  const bytes = new Uint8Array(loaded)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.length
  }
  return bytes
}

async function preInitGpu(): Promise<{ status: number; device: unknown }> {
  const nav = navigator as unknown as { gpu?: { requestAdapter(opts?: unknown): Promise<{ requestDevice(): Promise<unknown> } | null> } }
  if (!nav.gpu) {
    console.warn('[game] navigator.gpu missing — running without GPU')
    return { status: 2, device: null }
  }
  try {
    const adapter = await nav.gpu.requestAdapter({ powerPreference: 'low-power' })
    if (!adapter) throw new Error('no adapter')
    const device = await adapter.requestDevice()
    console.log('[game] GPU ready')
    return { status: 1, device }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    console.warn('[game] GPU init failed:', msg)
    return { status: 2, device: null }
  }
}

async function loadBuildInfo() {
  const el = document.getElementById('game-loading-build')
  if (!el) return
  try {
    const r = await fetch('/build-info.json', { cache: 'no-cache' })
    if (!r.ok) return
    const info = await r.json() as { short: string; built_at: string }
    el.textContent = `build: ${info.short} · ${info.built_at}`
  } catch (_) {}
}

async function main() {
  loadBuildInfo()
  const gpuPromise = preInitGpu()
  const wasmBytes = await streamWasmBytes('/game.wasm')
  const gpu = await gpuPromise

  // Handle tables for GPU resources — u32 handles from wasm map to real
  // GPU objects. Explicit lifetime for GPUBuffer (has .destroy());
  // shader/layout/pipeline are refcounted by the browser.
  const buffers = new Map<number, GPUBuffer>()
  const shaders = new Map<number, GPUShaderModule>()
  const bindGroupLayouts = new Map<number, GPUBindGroupLayout>()
  const bindGroups = new Map<number, GPUBindGroup>()
  const pipelineLayouts = new Map<number, GPUPipelineLayout>()
  const renderPipelines = new Map<number, GPURenderPipeline>()
  let nextHandle = 1
  const device = gpu.device as (GPUDevice | null)

  const COLOR_FORMATS = ['rgba8unorm', 'bgra8unorm'] as const
  const DEPTH_FORMATS = ['depth32float', 'depth24plus'] as const

  const imports: WebAssembly.Imports = {
    env: {
      seer_emit: (ptr: number, len: number) => {
        console.log(`[game] ${decodeString(ptr, len)}`)
      },
      seer_record_hotspot: (seq: number, size: number, align: number) => {
        console.log(`[game.hotspot] seq=${seq} size=${size} align=${align}`)
      },
      seer_record_gpu_event: (id: number, kind: number, size: number, labelPtr: number, labelLen: number) => {
        const kindName = kind === 1 ? 'buffer' : kind === 2 ? 'texture' : kind === 3 ? 'shader' : `?(${kind})`
        console.log(`[game.gpu] id=${id} kind=${kindName} size=${size} label=${decodeString(labelPtr, labelLen)}`)
      },
      seer_record_gpu_destroyed: (id: number) => {
        console.log(`[game.gpu.destroyed] id=${id}`)
      },
      seer_report_metric: (frame: number, heap: number, live: number, gpuBytes: number) => {
        if (frame % 60 === 0) {
          console.log(`[game.metric] frame=${frame} heap=${heap} gpu_live=${live} gpu_bytes=${gpuBytes}`)
        }
      },
      game_gpu_init: (_powerPref: number) => {
        // JS pre-init already ran. Idempotent no-op.
      },
      game_gpu_status: (): number => gpu.status,
      game_gpu_buffer_create: (size: number, usage: number, labelPtr: number, labelLen: number): number => {
        if (!device) return 0
        try {
          const label = decodeString(labelPtr, labelLen)
          const buf = device.createBuffer({ size, usage, label })
          const h = nextHandle++
          buffers.set(h, buf)
          return h
        } catch (e) {
          console.error('[game.gpu_buffer_create]', e)
          return 0
        }
      },
      game_gpu_buffer_write: (handle: number, dataPtr: number, dataLen: number) => {
        if (!device || !memory) return
        const buf = buffers.get(handle)
        if (!buf) return
        const view = new Uint8Array(memory.buffer, dataPtr, dataLen)
        device.queue.writeBuffer(buf, 0, view)
      },
      game_gpu_buffer_destroy: (handle: number) => {
        const buf = buffers.get(handle)
        if (!buf) return
        buf.destroy()
        buffers.delete(handle)
      },
      game_gpu_shader_module_create: (srcPtr: number, srcLen: number, labelPtr: number, labelLen: number): number => {
        if (!device) return 0
        try {
          const code = decodeString(srcPtr, srcLen)
          const label = decodeString(labelPtr, labelLen)
          const mod = device.createShaderModule({ code, label })
          const h = nextHandle++
          shaders.set(h, mod)
          return h
        } catch (e) {
          console.error('[game.gpu_shader_module_create]', e)
          return 0
        }
      },
      game_gpu_bind_group_layout_create_uniform: (labelPtr: number, labelLen: number): number => {
        if (!device) return 0
        try {
          const label = decodeString(labelPtr, labelLen)
          const bgl = device.createBindGroupLayout({
            label,
            entries: [{
              binding: 0,
              visibility: GPUShaderStage.VERTEX,
              buffer: { type: 'uniform' },
            }],
          })
          const h = nextHandle++
          bindGroupLayouts.set(h, bgl)
          return h
        } catch (e) {
          console.error('[game.gpu_bind_group_layout_create_uniform]', e)
          return 0
        }
      },
      game_gpu_bind_group_create: (layoutH: number, bufferH: number, labelPtr: number, labelLen: number): number => {
        if (!device) return 0
        const layout = bindGroupLayouts.get(layoutH)
        const buffer = buffers.get(bufferH)
        if (!layout || !buffer) return 0
        try {
          const label = decodeString(labelPtr, labelLen)
          const bg = device.createBindGroup({
            label,
            layout,
            entries: [{ binding: 0, resource: { buffer } }],
          })
          const h = nextHandle++
          bindGroups.set(h, bg)
          return h
        } catch (e) {
          console.error('[game.gpu_bind_group_create]', e)
          return 0
        }
      },
      game_gpu_pipeline_layout_create: (bgLayoutH: number, labelPtr: number, labelLen: number): number => {
        if (!device) return 0
        const bgl = bindGroupLayouts.get(bgLayoutH)
        if (!bgl) return 0
        try {
          const label = decodeString(labelPtr, labelLen)
          const pl = device.createPipelineLayout({
            label,
            bindGroupLayouts: [bgl],
          })
          const h = nextHandle++
          pipelineLayouts.set(h, pl)
          return h
        } catch (e) {
          console.error('[game.gpu_pipeline_layout_create]', e)
          return 0
        }
      },
      game_gpu_render_pipeline_create_cube: (
        pipelineLayoutH: number, shaderH: number, vertexStride: number, instanceStride: number,
        colorFormat: number, depthFormat: number, labelPtr: number, labelLen: number,
      ): number => {
        if (!device) return 0
        const pl = pipelineLayouts.get(pipelineLayoutH)
        const shader = shaders.get(shaderH)
        if (!pl || !shader) return 0
        const colorFmt = COLOR_FORMATS[colorFormat]
        const depthFmt = DEPTH_FORMATS[depthFormat]
        if (!colorFmt || !depthFmt) return 0
        try {
          const label = decodeString(labelPtr, labelLen)
          const pipeline = device.createRenderPipeline({
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
          const h = nextHandle++
          renderPipelines.set(h, pipeline)
          return h
        } catch (e) {
          console.error('[game.gpu_render_pipeline_create_cube]', e)
          return 0
        }
      },
    },
  }

  const { instance } = await WebAssembly.instantiate(wasmBytes, imports)
  memory = instance.exports.memory as WebAssembly.Memory

  const init = instance.exports.init as (() => void) | undefined
  const frame = instance.exports.frame as (() => number) | undefined
  if (typeof init !== 'function' || typeof frame !== 'function') {
    throw new Error(`game.wasm missing init/frame — exports: ${Object.keys(instance.exports).join(', ')}`)
  }

  init()
  document.body.classList.add('loaded')

  const loop = () => {
    const done = frame()
    if (done !== 0) return
    requestAnimationFrame(loop)
  }
  requestAnimationFrame(loop)
}

main().catch(e => {
  const msg = e instanceof Error ? e.message : String(e)
  if (loadingText) loadingText.textContent = `boot failed: ${msg}`
  console.error('[game] boot failed:', e)
})
