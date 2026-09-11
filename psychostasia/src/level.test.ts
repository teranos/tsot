import { describe, expect, test } from 'bun:test'
import { EmptySamplesError, loudness, rms } from './level'

describe('rms: how much sound is in one frame', () => {
  test('silence is zero', () => {
    expect(rms(new Float32Array(512))).toBe(0)
  })

  test('a constant signal is its own magnitude', () => {
    expect(rms(new Float32Array(512).fill(0.5))).toBeCloseTo(0.5, 6)
  })

  test('a full-scale sine is 1/sqrt(2)', () => {
    const n = 4800
    const sine = Float32Array.from({ length: n }, (_, i) => Math.sin((2 * Math.PI * 10 * i) / n))
    expect(rms(sine)).toBeCloseTo(Math.SQRT1_2, 4)
  })

  test('an empty frame is refused, typed', () => {
    expect(() => rms(new Float32Array(0))).toThrow(EmptySamplesError)
  })
})

describe('loudness: rms onto a 0..1 dB scale (-60 dB .. 0 dB)', () => {
  test('silence and anything under -60 dB is 0', () => {
    expect(loudness(0)).toBe(0)
    expect(loudness(0.0001)).toBe(0)
  })

  test('full scale is 1', () => {
    expect(loudness(1)).toBe(1)
  })

  test('-30 dB is halfway', () => {
    expect(loudness(10 ** (-30 / 20))).toBeCloseTo(0.5, 6)
  })
})
