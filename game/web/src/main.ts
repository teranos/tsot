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

// WASD input state — a u32 bitmask (W=1, A=2, S=4, D=8) maintained
// by window keydown/keyup listeners. Wasm polls via game_input_state.
let inputBits = 0
const KEY_W = 0x01
const KEY_A = 0x02
const KEY_S = 0x04
const KEY_D = 0x08
function keyBit(k: string): number {
  switch (k.toLowerCase()) {
    case 'w': return KEY_W
    case 'a': return KEY_A
    case 's': return KEY_S
    case 'd': return KEY_D
    default: return 0
  }
}
window.addEventListener('keydown', e => {
  const b = keyBit(e.key)
  if (b) { inputBits |= b; e.preventDefault() }
})
window.addEventListener('keyup', e => {
  const b = keyBit(e.key)
  if (b) { inputBits &= ~b; e.preventDefault() }
})

// Mobile D-pad — visible only on touchscreens via CSS @media (pointer: coarse).
// Touch (or mouse-down for testing on desktop) sets a WASD bit in the same
// inputBits bitmask keyboard uses; release clears it. Same env.* poll from
// game.wasm.
const dpad = document.getElementById('mobile-dpad')
if (dpad) {
  for (const btn of Array.from(dpad.querySelectorAll<HTMLElement>('button[data-key]'))) {
    const bit = keyBit(btn.getAttribute('data-key') || '')
    if (!bit) continue
    const on = (e: Event) => { e.preventDefault(); inputBits |= bit }
    const off = (e: Event) => { e.preventDefault(); inputBits &= ~bit }
    btn.addEventListener('touchstart', on, { passive: false })
    btn.addEventListener('touchend', off, { passive: false })
    btn.addEventListener('touchcancel', off, { passive: false })
    btn.addEventListener('mousedown', on)
    btn.addEventListener('mouseup', off)
    btn.addEventListener('mouseleave', off)
  }
}

// IndexedDB identity storage. Rust owns the bytes; JS owns the store.
// Pre-boot: try to load "self" so Rust's game_identity_load gets a
// synchronous answer from the cached value. If Rust asks and there's
// nothing, it generates via game_random_bytes and calls save.
const IDENTITY_DB = 'game'
const IDENTITY_STORE = 'identity'
const IDENTITY_KEY = 'self'
let identityBytes: Uint8Array | null = null

function openIdentityDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(IDENTITY_DB, 1)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains(IDENTITY_STORE)) {
        db.createObjectStore(IDENTITY_STORE)
      }
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error)
  })
}

async function loadIdentityFromDb(): Promise<Uint8Array | null> {
  const db = await openIdentityDb()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(IDENTITY_STORE, 'readonly')
    const getReq = tx.objectStore(IDENTITY_STORE).get(IDENTITY_KEY)
    getReq.onsuccess = () => {
      const val = getReq.result
      if (val instanceof Uint8Array) resolve(val)
      else if (val instanceof ArrayBuffer) resolve(new Uint8Array(val))
      else resolve(null)
    }
    getReq.onerror = () => reject(getReq.error)
  })
}

async function saveIdentityToDb(bytes: Uint8Array): Promise<void> {
  const db = await openIdentityDb()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(IDENTITY_STORE, 'readwrite')
    const putReq = tx.objectStore(IDENTITY_STORE).put(bytes, IDENTITY_KEY)
    putReq.onsuccess = () => resolve()
    putReq.onerror = () => reject(putReq.error)
  })
}

// Audio — non-spatial one-shot. `audioLoad` returns a handle
// synchronously; the actual fetch + decode runs async off-thread.
// Play before decode completes queues intent. Browsers require a user
// gesture (WASD keydown here) before AudioContext can produce sound.
type AudioSlot = {
  buffer: AudioBuffer | null
  source: AudioBufferSourceNode | null
  pendingPlay: { volume: number, loop: boolean } | null
  gain: GainNode | null
}
let audioCtx: AudioContext | null = null
let audioUnlocked = false
const audioSlots = new Map<number, AudioSlot>()
let nextAudioHandle = 1

function ensureAudioCtx(): AudioContext | null {
  if (audioCtx) return audioCtx
  try {
    audioCtx = new AudioContext()
  } catch (e) {
    console.warn('[game.audio] AudioContext unavailable:', e)
    return null
  }
  return audioCtx
}

function audioLoad(url: string): number {
  const ctx = ensureAudioCtx()
  if (!ctx) return 0
  const h = nextAudioHandle++
  const slot: AudioSlot = { buffer: null, source: null, pendingPlay: null, gain: null }
  audioSlots.set(h, slot)
  fetch(url)
    .then(r => {
      if (!r.ok) throw new Error(`fetch ${url}: ${r.status}`)
      return r.arrayBuffer()
    })
    .then(bytes => ctx.decodeAudioData(bytes))
    .then(buf => {
      slot.buffer = buf
      // If play was queued before decode finished, start it now.
      if (slot.pendingPlay) {
        const { volume, loop } = slot.pendingPlay
        slot.pendingPlay = null
        startAudioSlot(slot, volume, loop)
      }
    })
    .catch(e => console.warn('[game.audio] load failed:', e))
  return h
}

function startAudioSlot(slot: AudioSlot, volume: number, loop: boolean) {
  const ctx = audioCtx
  if (!ctx || !slot.buffer) return
  const source = ctx.createBufferSource()
  source.buffer = slot.buffer
  source.loop = loop
  const gain = ctx.createGain()
  gain.gain.value = volume
  source.connect(gain).connect(ctx.destination)
  source.start(0)
  slot.source = source
  slot.gain = gain
}

function audioPlay(handle: number, volume: number, loop: boolean) {
  const slot = audioSlots.get(handle)
  if (!slot) return
  // If context is still locked, buffer intent and wait for the unlock.
  if (!audioUnlocked) {
    slot.pendingPlay = { volume, loop }
    return
  }
  if (!slot.buffer) {
    slot.pendingPlay = { volume, loop }
    return
  }
  startAudioSlot(slot, volume, loop)
}

function audioStop(handle: number) {
  const slot = audioSlots.get(handle)
  if (!slot) return
  slot.pendingPlay = null
  if (slot.source) {
    try { slot.source.stop() } catch {}
    slot.source = null
  }
  audioSlots.delete(handle)
}

// First user gesture unlocks the AudioContext and flushes any
// queued play calls that were waiting on it.
function unlockAudioOnGesture() {
  if (audioUnlocked) return
  const ctx = ensureAudioCtx()
  if (!ctx) return
  ctx.resume().then(() => {
    audioUnlocked = true
    for (const slot of audioSlots.values()) {
      if (slot.pendingPlay && slot.buffer) {
        const { volume, loop } = slot.pendingPlay
        slot.pendingPlay = null
        startAudioSlot(slot, volume, loop)
      }
    }
  }).catch(e => console.warn('[game.audio] resume failed:', e))
}
window.addEventListener('keydown', unlockAudioOnGesture, { once: false })
window.addEventListener('pointerdown', unlockAudioOnGesture, { once: false })

// Remote-players proxy — thin WebSocket bridge to relaye's R16 WS
// gateway (see game/docs/relaye-game-gateway.md). Defaults to the
// deployed endpoint so game.sbvh.nl works out of the box; override
// with ?proxy=ws://... for local dev, or ?proxy=off to disable.
// Incoming messages are one GamePosition JSON each; we frame them
// length-prefixed (u32 LE + bytes) so Rust drains one buffer per
// tick and slices with parse_frames.
const DEFAULT_PROXY_WS = 'wss://relaye.sbvh.nl/ws/rave-positions/v1'
const proxyParam = new URLSearchParams(location.search).get('proxy')
const PROXY_WS_URL = proxyParam === 'off' ? '' : (proxyParam || DEFAULT_PROXY_WS)
let proxyWs: WebSocket | null = null
let proxyRxBuf: Uint8Array = new Uint8Array(0)

function connectProxy() {
  if (!PROXY_WS_URL) return
  try {
    proxyWs = new WebSocket(PROXY_WS_URL)
    proxyWs.binaryType = 'arraybuffer'
    proxyWs.onopen = () => console.log('[game.proxy] open', PROXY_WS_URL)
    proxyWs.onclose = () => console.log('[game.proxy] close')
    proxyWs.onerror = e => console.warn('[game.proxy] error', e)
    proxyWs.onmessage = ev => {
      let payload: Uint8Array
      if (typeof ev.data === 'string') {
        payload = new TextEncoder().encode(ev.data)
      } else if (ev.data instanceof ArrayBuffer) {
        payload = new Uint8Array(ev.data)
      } else {
        return
      }
      const len = payload.length
      const merged = new Uint8Array(proxyRxBuf.length + 4 + len)
      merged.set(proxyRxBuf)
      new DataView(merged.buffer).setUint32(proxyRxBuf.length, len, true)
      merged.set(payload, proxyRxBuf.length + 4)
      proxyRxBuf = merged
    }
  } catch (e) {
    console.warn('[game.proxy] connect failed:', e)
  }
}
connectProxy()

// Exclamation overlay — Rust computes clip-space coords above the NPC
// each frame and passes them here; JS maps to canvas pixels and shows
// a boxed "!" above that point. Repeated calls refresh the timeout so
// it lingers for a short tail after the overlap ends.
const bangEl = document.getElementById('game-bang')
let bangTimeout: ReturnType<typeof setTimeout> | null = null
function showBang(clipX: number, clipY: number) {
  if (!bangEl) return
  const canvas = document.getElementById('game-canvas')
  if (canvas) {
    const rect = canvas.getBoundingClientRect()
    const xPx = rect.left + (clipX + 1) * 0.5 * rect.width
    const yPx = rect.top + (1 - clipY) * 0.5 * rect.height
    bangEl.style.left = `${xPx}px`
    bangEl.style.top = `${yPx}px`
  }
  bangEl.classList.add('shown')
  if (bangTimeout) clearTimeout(bangTimeout)
  bangTimeout = setTimeout(() => bangEl.classList.remove('shown'), 1200)
}

async function main() {
  loadBuildInfo()
  const gpuPromise = preInitGpu()
  const identityPromise = loadIdentityFromDb().catch(e => {
    console.warn('[game] identity load failed:', e)
    return null
  })
  const wasmBytes = await streamWasmBytes('/game.wasm')
  const gpu = await gpuPromise
  identityBytes = await identityPromise

  // Handle tables for GPU resources — u32 handles from wasm map to real
  // GPU objects. Explicit lifetime for GPUBuffer (has .destroy());
  // shader/layout/pipeline are refcounted by the browser.
  const buffers = new Map<number, GPUBuffer>()
  const shaders = new Map<number, GPUShaderModule>()
  const bindGroupLayouts = new Map<number, GPUBindGroupLayout>()
  const bindGroups = new Map<number, GPUBindGroup>()
  const pipelineLayouts = new Map<number, GPUPipelineLayout>()
  const renderPipelines = new Map<number, GPURenderPipeline>()
  type RenderTarget = { context: GPUCanvasContext, depthView: GPUTextureView }
  const renderTargets = new Map<number, RenderTarget>()
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
      game_gpu_render_target_configure: (canvasIdPtr: number, canvasIdLen: number, colorFormat: number, depthFormat: number): number => {
        if (!device) return 0
        const colorFmt = COLOR_FORMATS[colorFormat]
        const depthFmt = DEPTH_FORMATS[depthFormat]
        if (!colorFmt || !depthFmt) return 0
        const canvasId = decodeString(canvasIdPtr, canvasIdLen)
        const canvas: HTMLCanvasElement | null = canvasId.startsWith('#')
          ? document.querySelector(canvasId)
          : document.getElementById(canvasId) as HTMLCanvasElement | null
        if (!canvas) return 0
        const ctx = canvas.getContext('webgpu') as GPUCanvasContext | null
        if (!ctx) return 0
        try {
          const dpr = window.devicePixelRatio || 1
          canvas.width = Math.max(1, Math.floor(canvas.clientWidth * dpr))
          canvas.height = Math.max(1, Math.floor(canvas.clientHeight * dpr))
          ctx.configure({ device, format: colorFmt, alphaMode: 'opaque' })
          const depthTex = device.createTexture({
            label: 'game.depth',
            size: [canvas.width, canvas.height, 1],
            format: depthFmt,
            usage: GPUTextureUsage.RENDER_ATTACHMENT,
          })
          const depthView = depthTex.createView({ label: 'game.depth.view' })
          const h = nextHandle++
          renderTargets.set(h, { context: ctx, depthView })
          return h
        } catch (e) {
          console.error('[game.gpu_render_target_configure]', e)
          return 0
        }
      },
      game_gpu_render_frame: (
        targetH: number, pipelineH: number, bindGroupH: number,
        vertexBufH: number, instanceBufH: number,
        vertexCount: number, instanceCount: number,
        clearR: number, clearG: number, clearB: number,
      ): number => {
        if (!device) return 1
        const target = renderTargets.get(targetH)
        const pipeline = renderPipelines.get(pipelineH)
        const bindGroup = bindGroups.get(bindGroupH)
        const vertexBuf = buffers.get(vertexBufH)
        const instanceBuf = buffers.get(instanceBufH)
        if (!target || !pipeline || !bindGroup || !vertexBuf || !instanceBuf) return 1
        try {
          const colorView = target.context.getCurrentTexture().createView({ label: 'game.frame.color' })
          const encoder = device.createCommandEncoder({ label: 'game.frame.encoder' })
          const pass = encoder.beginRenderPass({
            colorAttachments: [{
              view: colorView,
              clearValue: { r: clearR, g: clearG, b: clearB, a: 1.0 },
              loadOp: 'clear',
              storeOp: 'store',
            }],
            depthStencilAttachment: {
              view: target.depthView,
              depthClearValue: 1.0,
              depthLoadOp: 'clear',
              depthStoreOp: 'store',
            },
          })
          pass.setPipeline(pipeline)
          pass.setBindGroup(0, bindGroup)
          pass.setVertexBuffer(0, vertexBuf)
          pass.setVertexBuffer(1, instanceBuf)
          pass.draw(vertexCount, instanceCount)
          pass.end()
          device.queue.submit([encoder.finish()])
          return 0
        } catch (e) {
          console.error('[game.gpu_render_frame]', e)
          return 1
        }
      },
      game_input_state: (): number => inputBits,
      game_show_exclamation: (clipX: number, clipY: number) => showBang(clipX, clipY),
      game_identity_load: (outPtr: number): number => {
        if (!memory || !identityBytes || identityBytes.length !== 32) return 0
        new Uint8Array(memory.buffer, outPtr, 32).set(identityBytes)
        return 32
      },
      game_identity_save: (bytesPtr: number, bytesLen: number) => {
        if (!memory) return
        const copy = new Uint8Array(bytesLen)
        copy.set(new Uint8Array(memory.buffer, bytesPtr, bytesLen))
        identityBytes = copy
        saveIdentityToDb(copy).catch(e => console.warn('[game] identity save failed:', e))
      },
      game_random_bytes: (outPtr: number, outLen: number) => {
        if (!memory) return
        const view = new Uint8Array(memory.buffer, outPtr, outLen)
        crypto.getRandomValues(view)
      },
      game_peers_pending: (): number => proxyRxBuf.length,
      game_peers_recv: (outPtr: number, outLen: number): number => {
        if (!memory) return 0
        const n = Math.min(outLen, proxyRxBuf.length)
        if (n === 0) return 0
        new Uint8Array(memory.buffer, outPtr, n).set(proxyRxBuf.subarray(0, n))
        proxyRxBuf = proxyRxBuf.subarray(n)
        return n
      },
      game_self_publish: (bytesPtr: number, bytesLen: number) => {
        if (!memory) return
        if (!proxyWs || proxyWs.readyState !== WebSocket.OPEN) return
        const copy = new Uint8Array(bytesLen)
        copy.set(new Uint8Array(memory.buffer, bytesPtr, bytesLen))
        try { proxyWs.send(copy) } catch (e) { console.warn('[game.publish]', e) }
      },
      game_now_ms: (): number => Date.now(),
      // --- Audio ---
      // Non-spatial: single looped track for now. JS owns AudioContext
      // + AudioBufferSourceNode graph, Rust owns the handle lifetime.
      // Browser blocks AudioContext.resume() until first user gesture,
      // so we buffer "wanted to play" and start on the first WASD key.
      game_audio_load: (pathPtr: number, pathLen: number): number => {
        if (!memory) return 0
        const path = decodeString(pathPtr, pathLen)
        return audioLoad(path)
      },
      game_audio_play: (h: number, volX1000: number, loopFlag: number) => {
        audioPlay(h, volX1000 / 1000, loopFlag !== 0)
      },
      game_audio_stop: (h: number) => audioStop(h),
      // Rust-generated PCM samples → dumb WebAudio sink. Copy the
      // Float32Array out of wasm memory (memory can grow between calls,
      // and we want playback independent of wasm's lifetime), wrap in
      // an AudioBuffer, play once. Silent when the AudioContext hasn't
      // been unlocked yet (before first user gesture).
      game_audio_play_samples: (samplePtr: number, sampleCount: number, sampleRate: number) => {
        if (!memory) return
        const ctx = ensureAudioCtx()
        if (!ctx || !audioUnlocked || sampleCount === 0) return
        try {
          const view = new Float32Array(memory.buffer, samplePtr, sampleCount)
          const copy = new Float32Array(sampleCount)
          copy.set(view)
          const buf = ctx.createBuffer(1, sampleCount, sampleRate)
          buf.copyToChannel(copy, 0)
          const src = ctx.createBufferSource()
          src.buffer = buf
          src.connect(ctx.destination)
          src.start()
        } catch (e) {
          console.warn('[game.audio.play_samples]', e)
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
