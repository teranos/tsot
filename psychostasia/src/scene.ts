import type { Scale } from './judge'

// The beam's tilt at a fully light (or heavy) heart.
export const MAX_TILT_DEG = 18

// Heaven's sky shows through as the heart lightens, fully on a heaven
// verdict; any other verdict closes it.
export function heavenOpacity(s: Scale): number {
  if (s.verdict === 'heaven') return 1
  if (s.verdict !== null) return 0
  return Math.max(0, s.soul)
}

// The devil reveals itself once the soul is halfway to hell and is fully
// there on arrival, before the verdict. The opacity drives whatever draws
// the figure: SVG today, a 3D model later.
export const REVEAL_FROM = 0.5

function reveal(towards: number): number {
  return Math.min(1, Math.max(0, (towards - REVEAL_FROM) / (1 - REVEAL_FROM)))
}

export function devilOpacity(s: Scale): number {
  if (s.verdict === 'hell') return 1
  if (s.verdict !== null) return 0
  return reveal(-s.soul)
}

// The clips crawl in silence and race when it is loud.
export const MIN_PLAYBACK = 0.25
export const MAX_PLAYBACK = 2

export function playbackRate(loud: number): number {
  return MIN_PLAYBACK + (MAX_PLAYBACK - MIN_PLAYBACK) * Math.min(1, Math.max(0, loud))
}

// Hell's fire mirrors it: burns through as the heart grows heavy.
export function hellOpacity(s: Scale): number {
  if (s.verdict === 'hell') return 1
  if (s.verdict !== null) return 0
  return Math.max(0, -s.soul)
}
