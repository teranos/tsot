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
