import { describe, expect, test } from 'bun:test'
import {
  EARTH,
  EmptySpectrumError,
  HEAVEN,
  HELL,
  HOLD_S,
  MAX_INTERVAL_S,
  MIN_INTERVAL_S,
  type Scale,
  TUNING,
  color,
  flatness,
  judgeNow,
  keyVerdict,
  PUSH_SECONDS,
  push,
  scale,
  step,
  tick,
} from './judge'

describe('flatness: tonal (heaven-bound) vs noise (hell-bound)', () => {
  test('equal power in every bin is pure noise: 1', () => {
    expect(flatness(new Float32Array(1024).fill(0.3))).toBeCloseTo(1, 6)
  })

  test('all power in one bin is a pure tone: near 0', () => {
    const tone = new Float32Array(1024)
    tone[40] = 1
    expect(flatness(tone)).toBeLessThan(0.01)
  })

  test('an empty spectrum is refused, typed', () => {
    expect(() => flatness(new Float32Array(0))).toThrow(EmptySpectrumError)
  })
})

describe('step: the soul drifts with the sound', () => {
  const DT = 1 / 60

  test('silence sinks the soul toward hell', () => {
    expect(step(0.2, 0, 0.0, DT)).toBeLessThan(0.2)
  })

  test('silence sinks slower than loud noise', () => {
    expect(step(0, 0, 0.0, DT)).toBeGreaterThan(step(0, 1, 0.9, DT))
  })

  test('a loud tonal sound rises toward heaven', () => {
    expect(step(0, 0.8, 0.05, DT)).toBeGreaterThan(0)
  })

  test('a loud noisy sound sinks toward hell', () => {
    expect(step(0, 0.8, 0.9, DT)).toBeLessThan(0)
  })

  test('louder moves further', () => {
    expect(step(0, 1, 0.05, DT)).toBeGreaterThan(step(0, 0.5, 0.05, DT))
  })

  test('a lower silence threshold lets a quiet tone count', () => {
    expect(step(0, 0.1, 0.05, DT)).toBeLessThan(0)
    expect(step(0, 0.1, 0.05, DT, { ...TUNING, silence: 0.05 })).toBeGreaterThan(0)
  })

  test('a higher rate moves the soul further', () => {
    expect(step(0, 1, 0.05, DT, { ...TUNING, rate: 0.5 })).toBeGreaterThan(step(0, 1, 0.05, DT))
  })

  test('heaven and hell are the ends: the soul stays within -1..1', () => {
    expect(step(1, 1, 0.05, 10)).toBe(1)
    expect(step(-1, 1, 0.9, 10)).toBe(-1)
  })
})

describe('tick: judgment at most once per minute, at least once per 3 minutes', () => {
  const SILENT = [0, 0] as const
  const TONAL = [1, 0.05] as const
  const NOISE = [1, 0.9] as const

  function run(s: Scale, [loud, flat]: readonly [number, number], seconds: number): Scale {
    for (let t = 0; t < seconds; t += 0.5) s = tick(s, loud, flat, 0.5)
    return s
  }

  test('a soul that arrives early waits for the minute', () => {
    const s = run(scale(), TONAL, MIN_INTERVAL_S - 1)
    expect(s.soul).toBe(1)
    expect(s.verdict).toBeNull()
  })

  test('an arrived soul is judged once the minute has passed', () => {
    expect(run(scale(), TONAL, MIN_INTERVAL_S + 1).verdict).toBe('heaven')
    expect(run(scale(), NOISE, MIN_INTERVAL_S + 1).verdict).toBe('hell')
  })

  // Tonal and noise in turn: the soul sways but never arrives.
  function sway(s: Scale, seconds: number): Scale {
    let i = 0
    for (let t = 0; t < seconds; t += 0.5, i++) s = tick(s, ...(i % 2 === 0 ? TONAL : NOISE), 0.5)
    return s
  }

  test('a soul that never arrives is judged by 3 minutes, by where it leans', () => {
    let s = run(scale(), TONAL, 1)
    s = sway(s, MAX_INTERVAL_S)
    expect(s.verdict).toBe('heaven')
  })

  test('a soul that leans nowhere by 3 minutes is neither', () => {
    expect(sway(scale(), MAX_INTERVAL_S + 1).verdict).toBe('neither')
  })

  test('no judgment before 3 minutes unless the soul arrives', () => {
    expect(sway(scale(), MAX_INTERVAL_S - 1).verdict).toBeNull()
  })

  test('silence dominating sinks the soul to hell', () => {
    expect(run(scale(), SILENT, MIN_INTERVAL_S + 1).verdict).toBe('hell')
  })

  test('the verdict holds, then a new soul starts on earth', () => {
    let s = run(scale(), TONAL, MIN_INTERVAL_S + 1)
    expect(s.verdict).toBe('heaven')
    s = run(s, NOISE, HOLD_S - 2)
    expect(s.verdict).toBe('heaven')
    s = tick({ ...s, verdictAge: HOLD_S - 0.1 }, ...NOISE, 0.5)
    expect(s.verdict).toBeNull()
    expect(s.soul).toBe(0)
  })

  test('two judgments are never less than a minute apart', () => {
    let s = run(scale(), TONAL, MIN_INTERVAL_S + 1)
    const judgedAt: number[] = []
    let t = 0
    let prev = s.verdict
    for (; t < 4 * MAX_INTERVAL_S; t += 0.5) {
      s = tick(s, ...NOISE, 0.5)
      if (s.verdict !== null && prev === null) judgedAt.push(t)
      prev = s.verdict
    }
    expect(judgedAt.length).toBeGreaterThan(1)
    for (let i = 1; i < judgedAt.length; i++) {
      expect(judgedAt[i] - judgedAt[i - 1]).toBeGreaterThanOrEqual(MIN_INTERVAL_S)
    }
  })
})

describe('judgeNow: the performer sends the soul', () => {
  test('to heaven, at once, regardless of the minute', () => {
    const s = judgeNow(scale(), 'heaven')
    expect(s.verdict).toBe('heaven')
    expect(s.soul).toBe(1)
    expect(s.sinceJudgment).toBe(0)
  })

  test('to hell, whatever the heart weighed', () => {
    const s = judgeNow({ ...scale(), soul: 0.8 }, 'hell')
    expect(s.verdict).toBe('hell')
    expect(s.soul).toBe(-1)
  })

  test('the verdict then holds like any other', () => {
    const s = tick(judgeNow(scale(), 'hell'), 1, 0.05, HOLD_S - 1)
    expect(s.verdict).toBe('hell')
  })
})

describe('push: the arrows carry the soul there gradually', () => {
  test('holding up lifts the soul a step at a time', () => {
    const s = push(scale(), 1, 0.5)
    expect(s.soul).toBeGreaterThan(0)
    expect(s.soul).toBeLessThan(1)
    expect(s.verdict).toBeNull()
  })

  test('holding down sinks it', () => {
    expect(push(scale(), -1, 0.5).soul).toBeLessThan(0)
  })

  test('from earth, halfway takes half of PUSH_SECONDS', () => {
    expect(push(scale(), 1, PUSH_SECONDS / 2).soul).toBeCloseTo(0.5, 9)
  })

  test('arriving under the push is judged at once', () => {
    let s = scale()
    for (let i = 0; i < 100 && s.verdict === null; i++) s = push(s, 1, 0.1)
    expect(s.verdict).toBe('heaven')
    expect(s.sinceJudgment).toBe(0)
  })

  test('a verdict on display is not pushed', () => {
    const judged = judgeNow(scale(), 'hell')
    expect(push(judged, 1, 0.5)).toEqual(judged)
  })
})

describe('keyVerdict: which key sends the soul where', () => {
  test('h is heaven, H is hell', () => {
    expect(keyVerdict('h')).toBe('heaven')
    expect(keyVerdict('H')).toBe('hell')
  })

  test('the arrows are not instant: they send no verdict', () => {
    expect(keyVerdict('ArrowUp')).toBeNull()
    expect(keyVerdict('ArrowDown')).toBeNull()
  })

  test('any other key sends no one', () => {
    expect(keyVerdict('x')).toBeNull()
    expect(keyVerdict('Shift')).toBeNull()
  })
})

describe('color: where the soul is, the page is', () => {
  test('hell is hell', () => {
    expect(color(-1)).toBe(`rgb(${HELL.join(', ')})`)
  })

  test('earth is earth', () => {
    expect(color(0)).toBe(`rgb(${EARTH.join(', ')})`)
  })

  test('heaven is heaven', () => {
    expect(color(1)).toBe(`rgb(${HEAVEN.join(', ')})`)
  })
})
