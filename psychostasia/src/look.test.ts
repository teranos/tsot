import { describe, expect, test } from 'bun:test'
import { DEFAULT_LOOK, LOOK_PARAMS, LookParseError, parseLook } from './look'

describe('LOOK_PARAMS: every control is well formed', () => {
  test('each default sits within its range', () => {
    for (const p of LOOK_PARAMS) {
      expect(p.default).toBeGreaterThanOrEqual(p.min)
      expect(p.default).toBeLessThanOrEqual(p.max)
    }
  })

  test('keys are unique', () => {
    const keys = LOOK_PARAMS.map(p => p.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  test('DEFAULT_LOOK holds every default', () => {
    for (const p of LOOK_PARAMS) expect(DEFAULT_LOOK[p.key]).toBe(p.default)
  })
})

describe('parseLook: a saved look comes back', () => {
  test('nothing saved is the default look', () => {
    expect(parseLook(null)).toEqual(DEFAULT_LOOK)
  })

  test('saved values override the defaults, the rest stay default', () => {
    const [first, second] = LOOK_PARAMS
    const look = parseLook(JSON.stringify({ [first.key]: first.min }))
    expect(look[first.key]).toBe(first.min)
    expect(look[second.key]).toBe(second.default)
  })

  test('values out of range are pulled back into range', () => {
    const p = LOOK_PARAMS[0]
    expect(parseLook(JSON.stringify({ [p.key]: p.max + 1000 }))[p.key]).toBe(p.max)
  })

  test('keys no control knows are dropped', () => {
    expect(parseLook(JSON.stringify({ nonsense: 3 }))).toEqual(DEFAULT_LOOK)
  })

  test('a broken save is refused, typed, not silently reset', () => {
    expect(() => parseLook('{not json')).toThrow(LookParseError)
    expect(() => parseLook('[1,2]')).toThrow(LookParseError)
  })
})
