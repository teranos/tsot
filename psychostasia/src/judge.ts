// Tonal sound rises to heaven, noise sinks to hell; neither is better.
// Flatness is the measure: geometric mean over arithmetic mean of the
// power spectrum.

export class EmptySpectrumError extends Error {
  constructor() {
    super('flatness of an empty spectrum: no frequency bins to judge')
    this.name = 'EmptySpectrumError'
  }
}

const EPS = 1e-12

export function flatness(power: Float32Array): number {
  if (power.length === 0) throw new EmptySpectrumError()
  let logSum = 0
  let sum = 0
  for (const p of power) {
    const q = p + EPS
    logSum += Math.log(q)
    sum += q
  }
  return Math.exp(logSum / power.length) / (sum / power.length)
}

// Loudness under this is silence, which weighs the heart too.
export const SILENCE = 0.15
// Silence sinks the soul at this fraction of full-loudness noise.
export const SILENCE_PULL = 0.5
// Flatness under this is tonal (heaven-bound), at or over is noise.
export const FLATNESS_SPLIT = 0.25
// Soul travel per second at full loudness; -1 is hell, 1 is heaven.
export const RATE = 0.05

// The page's sliders override these live.
export interface Tuning {
  silence: number
  flatnessSplit: number
  rate: number
}
export const TUNING: Tuning = { silence: SILENCE, flatnessSplit: FLATNESS_SPLIT, rate: RATE }

export function step(soul: number, loud: number, flat: number, dt: number, t: Tuning = TUNING): number {
  const pull = loud < t.silence ? -SILENCE_PULL : flat < t.flatnessSplit ? loud : -loud
  return Math.min(1, Math.max(-1, soul + pull * t.rate * dt))
}

// h and H judge at once; the arrows drift instead (see push).
export function keyVerdict(key: string): 'heaven' | 'hell' | null {
  if (key === 'h') return 'heaven'
  if (key === 'H') return 'hell'
  return null
}

// Holding an arrow carries the soul from earth to an end in this long;
// arriving under the push is judged at once, outside the minute rule.
export const PUSH_SECONDS = 3

export function push(s: Scale, dir: 1 | -1, dt: number): Scale {
  if (s.verdict !== null) return s
  const soul = Math.min(1, Math.max(-1, s.soul + (dir * dt) / PUSH_SECONDS))
  if (Math.abs(soul) === 1) return judgeNow(s, soul > 0 ? 'heaven' : 'hell')
  return { ...s, soul }
}

// Judgment resolves on its own: an arrived soul is judged once a minute
// has passed; any soul is judged by 3 minutes, by where it leans.
export const MIN_INTERVAL_S = 60
export const MAX_INTERVAL_S = 180
// How long a verdict stays on display before a new soul starts on earth.
export const HOLD_S = 10

export type Place = 'heaven' | 'hell' | 'neither'

export interface Scale {
  soul: number
  sinceJudgment: number
  verdict: Place | null
  verdictAge: number
}

export function scale(): Scale {
  return { soul: 0, sinceJudgment: 0, verdict: null, verdictAge: 0 }
}

export function tick(s: Scale, loud: number, flat: number, dt: number, t: Tuning = TUNING): Scale {
  const sinceJudgment = s.sinceJudgment + dt
  if (s.verdict !== null) {
    const verdictAge = s.verdictAge + dt
    if (verdictAge >= HOLD_S) return { soul: 0, sinceJudgment, verdict: null, verdictAge: 0 }
    return { ...s, sinceJudgment, verdictAge }
  }
  const soul = step(s.soul, loud, flat, dt, t)
  const arrived = Math.abs(soul) === 1 && sinceJudgment >= MIN_INTERVAL_S
  if (arrived || sinceJudgment >= MAX_INTERVAL_S) {
    const verdict: Place = soul > 0 ? 'heaven' : soul < 0 ? 'hell' : 'neither'
    return { soul, sinceJudgment: 0, verdict, verdictAge: 0 }
  }
  return { soul, sinceJudgment, verdict: null, verdictAge: 0 }
}

// The performer sends the soul at once, outside the minute rule; the
// verdict then holds and the clock restarts like any judgment.
export function judgeNow(_s: Scale, p: 'heaven' | 'hell'): Scale {
  return { soul: p === 'heaven' ? 1 : -1, sinceJudgment: 0, verdict: p, verdictAge: 0 }
}

export function verdictSoul(p: Place): number {
  return p === 'heaven' ? 1 : p === 'hell' ? -1 : 0
}

type Rgb = readonly [number, number, number]
export const HELL: Rgb = [150, 12, 0]
export const EARTH: Rgb = [40, 40, 44]
export const HEAVEN: Rgb = [255, 244, 214]

export function color(soul: number): string {
  const to = soul < 0 ? HELL : HEAVEN
  const t = Math.abs(soul)
  const c = EARTH.map((f, i) => Math.round(f + (to[i] - f) * t))
  return `rgb(${c.join(', ')})`
}
