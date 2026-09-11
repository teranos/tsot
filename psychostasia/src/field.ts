import type { Scale } from './judge'
import type { Look } from './look'
import { heavenOpacity } from './scene'

// How the sound drives heaven's field, within the look: tonal sound opens
// more mirrors, loudness races and pulses the tunnel, and the field bleeds
// in as the heart lightens.

export interface FieldParams {
  segments: number
  speed: number
  pulse: number
  alpha: number
}

const clamp01 = (x: number) => Math.min(1, Math.max(0, x))

export function fieldParams(loud: number, flat: number, s: Scale, look: Look): FieldParams {
  const l = clamp01(loud)
  return {
    segments: Math.round(look.segMax - (look.segMax - look.segMin) * clamp01(flat)),
    speed: look.speedBase + look.speedLoud * l,
    pulse: look.pulseGain * l,
    alpha: heavenOpacity(s),
  }
}
