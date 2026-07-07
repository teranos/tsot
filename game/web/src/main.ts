// game.sbvh.nl bootstrap. Fetches /game.wasm, wires the seven env.*
// imports, drives requestAnimationFrame frame loop.
//
// Encapsulated GPU init: JS runs navigator.gpu.requestAdapter →
// requestDevice BEFORE wasm.init(). By the time wasm reads
// game_gpu_status, it's Ready (or Unavailable) — never Pending.

let memory: WebAssembly.Memory | null = null

function decodeString(ptr: number, len: number): string {
  if (!memory) return ''
  const bytes = new Uint8Array(memory.buffer, ptr, len)
  return new TextDecoder('utf-8').decode(bytes)
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

async function main() {
  const [wasmRes, gpu] = await Promise.all([
    fetch('/game.wasm'),
    preInitGpu(),
  ])
  if (!wasmRes.ok) {
    throw new Error(`game.wasm fetch → HTTP ${wasmRes.status}`)
  }
  const wasmBytes = await wasmRes.arrayBuffer()

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

  const loop = () => {
    const done = frame()
    if (done !== 0) return
    requestAnimationFrame(loop)
  }
  requestAnimationFrame(loop)
}

main().catch(e => {
  const msg = e instanceof Error ? e.message : String(e)
  document.body.textContent = `[game] boot failed: ${msg}`
  console.error('[game] boot failed:', e)
})
