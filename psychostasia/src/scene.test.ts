import { describe, expect, test } from 'bun:test'
import { scale } from './judge'
import {
  MAX_PLAYBACK,
  MIN_PLAYBACK,
  REVEAL_FROM,
  devilOpacity,
  heavenOpacity,
  hellOpacity,
  playbackRate,
} from './scene'

describe('devilOpacity: the devil reveals itself before hell', () => {
  test('fades in as the heart grows heavy past the reveal point', () => {
    expect(devilOpacity({ ...scale(), soul: -REVEAL_FROM })).toBe(0)
    expect(devilOpacity({ ...scale(), soul: -(REVEAL_FROM + 1) / 2 })).toBeCloseTo(0.5, 9)
    expect(devilOpacity({ ...scale(), soul: -1 })).toBe(1)
  })

  test('there through a hell verdict, gone on any other', () => {
    expect(devilOpacity({ ...scale(), soul: -1, verdict: 'hell' })).toBe(1)
    expect(devilOpacity({ ...scale(), soul: -0.9, verdict: 'heaven' })).toBe(0)
  })

  test('a light heart sees no devil', () => {
    expect(devilOpacity({ ...scale(), soul: 0.9 })).toBe(0)
  })
})

describe('playbackRate: the clips move with the sound', () => {
  test('silence slows them to a crawl', () => {
    expect(playbackRate(0)).toBe(MIN_PLAYBACK)
  })

  test('full loudness runs them fast', () => {
    expect(playbackRate(1)).toBe(MAX_PLAYBACK)
  })

  test('louder is faster', () => {
    expect(playbackRate(0.6)).toBeGreaterThan(playbackRate(0.3))
  })
})

describe('hellOpacity: hell burns through as the heart grows heavy', () => {
  test('earth and a light heart show no hell', () => {
    expect(hellOpacity({ ...scale(), soul: 0 })).toBe(0)
    expect(hellOpacity({ ...scale(), soul: 0.7 })).toBe(0)
  })

  test('a heavying heart lets hell through in proportion', () => {
    expect(hellOpacity({ ...scale(), soul: -0.4 })).toBeCloseTo(0.4, 9)
  })

  test('a hell verdict is all hell', () => {
    expect(hellOpacity({ ...scale(), soul: -1, verdict: 'hell' })).toBe(1)
  })

  test('a heaven or neither verdict shows no hell', () => {
    expect(hellOpacity({ ...scale(), soul: 1, verdict: 'heaven' })).toBe(0)
    expect(hellOpacity({ ...scale(), soul: -0.3, verdict: 'neither' })).toBe(0)
  })
})

describe('heavenOpacity: heaven shows through as the heart lightens', () => {
  test('earth and a heavy heart show no heaven', () => {
    expect(heavenOpacity({ ...scale(), soul: 0 })).toBe(0)
    expect(heavenOpacity({ ...scale(), soul: -0.7 })).toBe(0)
  })

  test('a lightening heart lets heaven through in proportion', () => {
    expect(heavenOpacity({ ...scale(), soul: 0.4 })).toBeCloseTo(0.4, 9)
  })

  test('a heaven verdict is all heaven', () => {
    expect(heavenOpacity({ ...scale(), soul: 1, verdict: 'heaven' })).toBe(1)
  })

  test('a hell or neither verdict shows no heaven', () => {
    expect(heavenOpacity({ ...scale(), soul: -1, verdict: 'hell' })).toBe(0)
    expect(heavenOpacity({ ...scale(), soul: 0.3, verdict: 'neither' })).toBe(0)
  })
})
