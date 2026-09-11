import { describe, expect, test } from 'bun:test'
import { fieldParams } from './field'
import { scale } from './judge'
import { DEFAULT_LOOK } from './look'

describe('fieldParams: the sound drives heaven’s field, within the look', () => {
  const earth = scale()
  const look = DEFAULT_LOOK

  test('a pure tone opens the tone mirror count, noise the noise count', () => {
    expect(fieldParams(0.5, 0, earth, look).segments).toBe(look.segMax)
    expect(fieldParams(0.5, 1, earth, look).segments).toBe(look.segMin)
  })

  test('mirrors come in whole numbers', () => {
    expect(Number.isInteger(fieldParams(0.5, 0.37, earth, look).segments)).toBe(true)
  })

  test('louder races the tunnel faster and pulses harder', () => {
    const quiet = fieldParams(0.1, 0.2, earth, look)
    const loud = fieldParams(0.9, 0.2, earth, look)
    expect(loud.speed).toBeGreaterThan(quiet.speed)
    expect(loud.pulse).toBeGreaterThan(quiet.pulse)
  })

  test('silence drifts at the drift speed', () => {
    expect(fieldParams(0, 0.5, earth, look).speed).toBe(look.speedBase)
  })

  test('the look scales the response', () => {
    const wild = { ...look, speedLoud: look.speedLoud * 2, pulseGain: look.pulseGain * 2 }
    expect(fieldParams(1, 0.2, earth, wild).speed).toBeGreaterThan(fieldParams(1, 0.2, earth, look).speed)
    expect(fieldParams(1, 0.2, earth, wild).pulse).toBeCloseTo(fieldParams(1, 0.2, earth, look).pulse * 2, 9)
  })

  test('the field bleeds in as the heart lightens, all of it at a heaven verdict', () => {
    expect(fieldParams(0.5, 0.2, { ...earth, soul: 0 }, look).alpha).toBe(0)
    expect(fieldParams(0.5, 0.2, { ...earth, soul: 0.6 }, look).alpha).toBeCloseTo(0.6, 9)
    expect(fieldParams(0.5, 0.2, { ...earth, soul: 1, verdict: 'heaven' }, look).alpha).toBe(1)
    expect(fieldParams(0.5, 0.2, { ...earth, soul: -1, verdict: 'hell' }, look).alpha).toBe(0)
  })
})
