import { configureGlyphs, glyphRun } from '@jsr/qntx__glyphs'
import { fieldParams } from './field'
import {
  MAX_INTERVAL_S,
  MIN_INTERVAL_S,
  color,
  flatness,
  judgeNow,
  keyVerdict,
  push,
  scale,
  tick,
  verdictSoul,
} from './judge'
import { loudness, rms } from './level'
import { DEFAULT_LOOK, LOOK_PARAMS, type Look, parseLook } from './look'
import { createScale3d } from './scale3d'
import { devilOpacity, hellOpacity, playbackRate } from './scene'
import { tweakGlyph } from './tweak'

// Judge only the band a voice and most instruments live in; the bins
// above are mic self-noise and would read every sound as tonal.
const BAND_LO_HZ = 100
const BAND_HI_HZ = 8000
// Longest step one frame may take: a return from a hidden tab must not
// jump the soul, but a slow frame rate must not slow it either.
const MAX_FRAME_S = 0.5
const LOOK_STORE = 'psychostasia.look'

class MissingElementError extends Error {
  constructor(id: string) {
    super(`#${id} is not in index.html`)
    this.name = 'MissingElementError'
  }
}

// A throw while the module loads only reaches the console, so every
// failure writes itself onto the page, even with #error missing.
function showError(where: string, e: unknown) {
  const text = e instanceof Error ? `${e.name}: ${e.message}` : String(e)
  let box = document.getElementById('error')
  if (!box) {
    box = document.createElement('pre')
    box.id = 'error'
    box.style.cssText = 'position:fixed;top:16px;left:16px;z-index:10;background:#000;padding:12px'
    document.body.append(box)
  }
  box.textContent = `${where}\n${text}`
  box.hidden = false
}

function el<T>(id: string): T {
  const found = document.getElementById(id)
  if (!found) {
    const e = new MissingElementError(id)
    showError('boot', e)
    throw e
  }
  return found as unknown as T
}

window.addEventListener('error', ev => showError('uncaught', ev.error ?? ev.message))
window.addEventListener('unhandledrejection', ev => showError('unhandled rejection', ev.reason))

const held = el<HTMLDivElement>('held')
const readout = el<HTMLPreElement>('readout')
const place = el<HTMLDivElement>('place')
const devil = el<SVGGElement>('devil')
const hell = el<HTMLVideoElement>('hell')
const panel = el<HTMLDivElement>('controls-panel')

hell.addEventListener('error', () =>
  showError(`hell video failed (${hell.src})`, hell.error ? `MediaError ${hell.error.code}: ${hell.error.message}` : 'unknown'),
)
hell.play().catch(e => showError('hell video would not play', e))

const scale3d = createScale3d(el<HTMLCanvasElement>('scale3d'), showError)

// The look: every adjustable number, saved in this browser.
function loadLook(): Look {
  let saved: string | null = null
  try {
    saved = localStorage.getItem(LOOK_STORE)
  } catch (e) {
    showError('look: browser storage unavailable, using the default look', e)
  }
  try {
    return parseLook(saved)
  } catch (e) {
    showError('look: saved look refused, using the default look (the save stays until a tweak changes)', e)
    return { ...DEFAULT_LOOK }
  }
}
const look = loadLook()
function lookChanged() {
  try {
    localStorage.setItem(LOOK_STORE, JSON.stringify(look))
  } catch (e) {
    showError('look: could not save to browser storage', e)
  }
  scale3d.setLook(look)
}
lookChanged()

// The glyph tray, with tweak as its first glyph. The resting dot is doubled
// from the package's 10px default, as its own examples do: a 10px dot on a
// dark piece is hard to aim at.
configureGlyphs({ dotGeometry: { minWidth: 20, minHeight: 20 } })
glyphRun.init()
glyphRun.add(
  tweakGlyph({
    params: LOOK_PARAMS,
    values: look,
    onChange: lookChanged,
    onError: showError,
    onReset: () => {
      try {
        localStorage.removeItem(LOOK_STORE)
      } catch (e) {
        showError('look: could not clear the saved look', e)
        return
      }
      location.reload()
    },
  }),
)

async function start() {
  let stream: MediaStream
  try {
    // Browser voice processing would erase exactly the noise we judge.
    stream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
    })
  } catch (e) {
    showError('mic refused (getUserMedia)', e)
    return
  }
  const ctx = new AudioContext()
  const analyser = ctx.createAnalyser()
  analyser.fftSize = 2048
  analyser.smoothingTimeConstant = 0.5
  ctx.createMediaStreamSource(stream).connect(analyser)
  // Autoplay policy may hold the context until a gesture.
  const wake = () => ctx.resume().catch(e => showError('audio resume failed', e))
  window.addEventListener('pointerdown', wake)
  window.addEventListener('keydown', wake)
  wake()
  run(analyser, ctx, stream.getAudioTracks()[0])
}

function run(analyser: AnalyserNode, ctx: AudioContext, track: MediaStreamTrack) {
  const wave = new Float32Array(analyser.fftSize)
  const db = new Float32Array(analyser.frequencyBinCount)
  const hzPerBin = ctx.sampleRate / analyser.fftSize
  const lo = Math.ceil(BAND_LO_HZ / hzPerBin)
  const hi = Math.min(db.length, Math.floor(BAND_HI_HZ / hzPerBin))
  const power = new Float32Array(hi - lo)
  let s = scale()
  let last = performance.now()

  // Held arrow: +1 drifts toward heaven, -1 toward hell, 0 none.
  let pushing: 1 | -1 | 0 = 0
  window.addEventListener('keydown', ev => {
    // Keys typed into a tweak belong to the tweak.
    if (ev.target instanceof HTMLInputElement) return
    if (ev.key === 'c') {
      panel.hidden = !panel.hidden
      return
    }
    if (ev.key === 'ArrowUp') pushing = 1
    if (ev.key === 'ArrowDown') pushing = -1
    const p = keyVerdict(ev.key)
    if (p) s = judgeNow(s, p)
  })
  window.addEventListener('keyup', ev => {
    if ((ev.key === 'ArrowUp' && pushing === 1) || (ev.key === 'ArrowDown' && pushing === -1)) pushing = 0
  })

  const frame = (now: number) => {
    try {
      const dt = Math.min(MAX_FRAME_S, (now - last) / 1000)
      last = now
      // A held context hands over silence; judging it would send every
      // soul to hell before the mic was ever heard.
      if (ctx.state !== 'running') {
        held.textContent =
          `audio ${ctx.state}: the browser holds audio until a click or key press.\n` +
          `kiosk: launch Chrome with --autoplay-policy=no-user-gesture-required`
        held.hidden = false
        requestAnimationFrame(frame)
        return
      }
      held.hidden = true
      analyser.getFloatTimeDomainData(wave)
      analyser.getFloatFrequencyData(db)
      for (let i = lo; i < hi; i++) power[i - lo] = 10 ** (db[i] / 10)
      const loud = loudness(rms(wave))
      const flat = flatness(power)
      s = tick(s, loud, flat, dt, look)
      if (pushing !== 0) s = push(s, pushing, dt)
      document.body.style.background = color(s.verdict ? verdictSoul(s.verdict) : s.soul)
      place.textContent = s.verdict ? s.verdict.toUpperCase() : 'EARTH'
      scale3d.setSoul(s.soul)
      scale3d.setField(fieldParams(loud, flat, s, look))
      hell.style.opacity = String(hellOpacity(s))
      hell.playbackRate = playbackRate(loud)
      devil.style.opacity = String(devilOpacity(s))
      const sound = loud < look.silence ? 'silent → hell' : flat < look.flatnessSplit ? 'tonal → heaven' : 'noise → hell'
      let peak = 0
      for (const x of wave) peak = Math.max(peak, Math.abs(x))
      readout.textContent = [
        `mic      ${track.label || '(no label)'}  ${track.readyState}${track.muted ? '  MUTED' : ''}`,
        `audio    ${ctx.state} @ ${ctx.sampleRate} Hz  peak ${peak.toFixed(4)}`,
        `loudness ${loud.toFixed(3)}  (silent under ${look.silence})`,
        `flatness ${flat.toFixed(3)}  (tonal under ${look.flatnessSplit})`,
        `sound    ${sound}`,
        `soul     ${s.soul.toFixed(3)}  (-1 hell, 1 heaven)`,
        `keys     h heaven now, H hell now, hold ↑/↓ to drift, c hides this readout`,
        `tweak    the dot in the tray at the right edge`,
        `judgment possible in ${Math.max(0, MIN_INTERVAL_S - s.sinceJudgment).toFixed(0)}s, forced in ${Math.max(0, MAX_INTERVAL_S - s.sinceJudgment).toFixed(0)}s`,
      ].join('\n')
    } catch (e) {
      showError('frame loop stopped', e)
      return
    }
    requestAnimationFrame(frame)
  }
  requestAnimationFrame(frame)
}

start()
document.documentElement.dataset.booted = '1'
