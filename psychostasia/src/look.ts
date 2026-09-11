// Every adjustable number of the piece, as one flat "look": the page
// builds a control per entry, saves the look in the browser, and shows it
// as JSON so a look can be read, copied and restored exactly.

export interface LookParam {
  key: string
  label: string
  group: string
  min: number
  max: number
  step: number
  default: number
}

export const LOOK_PARAMS = [
  { key: 'silence', label: 'silent under (loudness)', group: 'judgment', min: 0, max: 0.5, step: 0.01, default: 0.15 },
  { key: 'flatnessSplit', label: 'tonal under (flatness)', group: 'judgment', min: 0, max: 1, step: 0.01, default: 0.25 },
  { key: 'rate', label: 'soul speed', group: 'judgment', min: 0.01, max: 0.5, step: 0.01, default: 0.05 },

  { key: 'segMin', label: 'mirrors on noise', group: 'heaven: sound', min: 2, max: 24, step: 1, default: 5 },
  { key: 'segMax', label: 'mirrors on a tone', group: 'heaven: sound', min: 2, max: 24, step: 1, default: 16 },
  { key: 'speedBase', label: 'drift speed', group: 'heaven: sound', min: 0, max: 2, step: 0.01, default: 0.15 },
  { key: 'speedLoud', label: 'speed from loudness', group: 'heaven: sound', min: 0, max: 5, step: 0.01, default: 1.5 },
  { key: 'pulseGain', label: 'pulse from loudness', group: 'heaven: sound', min: 0, max: 3, step: 0.01, default: 1 },

  { key: 'folds', label: 'fractal folds', group: 'heaven: shape', min: 1, max: 6, step: 1, default: 3 },
  { key: 'foldX', label: 'fold x', group: 'heaven: shape', min: 0, max: 2, step: 0.01, default: 0.9 },
  { key: 'foldY', label: 'fold y', group: 'heaven: shape', min: 0, max: 2, step: 0.01, default: 0.6 },
  { key: 'wobble', label: 'fold wobble', group: 'heaven: shape', min: 0, max: 1, step: 0.01, default: 0.15 },
  { key: 'zoom', label: 'zoom', group: 'heaven: shape', min: 0.2, max: 5, step: 0.01, default: 1 },
  { key: 'ringFreq', label: 'tunnel rings', group: 'heaven: shape', min: 0, max: 20, step: 0.1, default: 6 },
  { key: 'ringDepth', label: 'ring contrast', group: 'heaven: shape', min: 0, max: 1, step: 0.01, default: 0.4 },

  { key: 'hue', label: 'hue', group: 'heaven: colour', min: 0, max: 1, step: 0.01, default: 0 },
  { key: 'colorSpeed', label: 'colour cycling', group: 'heaven: colour', min: 0, max: 2, step: 0.01, default: 0.2 },
  { key: 'spread', label: 'colour spread', group: 'heaven: colour', min: 0, max: 2, step: 0.01, default: 0.3 },
  { key: 'brightness', label: 'brightness', group: 'heaven: colour', min: 0, max: 3, step: 0.01, default: 1 },
  { key: 'core', label: 'centre glow', group: 'heaven: colour', min: 0, max: 1, step: 0.01, default: 0.15 },

  { key: 'tilt', label: 'beam tilt (°)', group: 'scale', min: 0, max: 45, step: 1, default: 18 },
  { key: 'yawMax', label: 'turn from cursor (°)', group: 'scale', min: 0, max: 90, step: 1, default: 30 },
] as const satisfies readonly LookParam[]

export type LookKey = (typeof LOOK_PARAMS)[number]['key']
export type Look = Record<LookKey, number>

export const DEFAULT_LOOK: Look = Object.fromEntries(LOOK_PARAMS.map(p => [p.key, p.default])) as Look

export class LookParseError extends Error {
  constructor(why: string) {
    super(`saved look is unreadable: ${why}`)
    this.name = 'LookParseError'
  }
}

// A saved look (JSON) over the defaults: known keys only, clamped to range.
export function parseLook(text: string | null): Look {
  const look: Look = { ...DEFAULT_LOOK }
  if (text === null) return look
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch (e) {
    throw new LookParseError(String(e))
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) throw new LookParseError('not a JSON object')
  for (const p of LOOK_PARAMS) {
    const v = (raw as Record<string, unknown>)[p.key]
    if (typeof v === 'number' && Number.isFinite(v)) look[p.key] = Math.min(p.max, Math.max(p.min, v))
  }
  return look
}
