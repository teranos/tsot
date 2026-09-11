export class EmptySamplesError extends Error {
  constructor() {
    super('rms of an empty frame: the analyser handed over 0 samples')
    this.name = 'EmptySamplesError'
  }
}

export function rms(samples: Float32Array): number {
  if (samples.length === 0) throw new EmptySamplesError()
  let sum = 0
  for (const s of samples) sum += s * s
  return Math.sqrt(sum / samples.length)
}

// Below this the page treats the mic as silent; 0 dB is full scale.
const FLOOR_DB = -60

export function loudness(r: number): number {
  if (r <= 0) return 0
  const db = 20 * Math.log10(r)
  return Math.min(1, Math.max(0, (db - FLOOR_DB) / -FLOOR_DB))
}
