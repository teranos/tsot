import { describe, expect, test } from 'bun:test'
import { MAX_YAW, swing, yaw } from './rig'

describe('yaw: the scale turns away from the cursor, left and right only', () => {
  const WIDTH = 1000

  test('cursor in the middle, facing straight on', () => {
    expect(yaw(500, WIDTH)).toBe(0)
  })

  test('cursor at the left edge turns it fully right', () => {
    expect(yaw(0, WIDTH)).toBe(MAX_YAW)
  })

  test('cursor at the right edge turns it fully left', () => {
    expect(yaw(1000, WIDTH)).toBe(-MAX_YAW)
  })

  test('halfway out turns halfway', () => {
    expect(yaw(250, WIDTH)).toBeCloseTo(MAX_YAW / 2, 9)
  })

  test('the limit can be set', () => {
    expect(yaw(0, WIDTH, 0.2)).toBe(0.2)
  })

  test('past the edges stays at the limit', () => {
    expect(yaw(-200, WIDTH)).toBe(MAX_YAW)
    expect(yaw(1300, WIDTH)).toBe(-MAX_YAW)
  })
})

// The brass scale seen from the front: x runs left to right, y is up.
const PIVOT = { x: 0, y: 21 }
const LEFT_HOOK = { x: -11, y: 18 }
const RIGHT_HOOK = { x: 11, y: 18 }

describe('swing: a hook turning with the beam about its pivot', () => {
  test('no angle, no move', () => {
    const p = swing(LEFT_HOOK, PIVOT, 0)
    expect(p.x).toBeCloseTo(LEFT_HOOK.x, 9)
    expect(p.y).toBeCloseTo(LEFT_HOOK.y, 9)
  })

  test('a positive angle raises the left (heart) hook', () => {
    expect(swing(LEFT_HOOK, PIVOT, 0.2).y).toBeGreaterThan(LEFT_HOOK.y)
  })

  test('a positive angle lowers the right (feather) hook', () => {
    expect(swing(RIGHT_HOOK, PIVOT, 0.2).y).toBeLessThan(RIGHT_HOOK.y)
  })

  test('the hook keeps its distance to the pivot', () => {
    const before = Math.hypot(LEFT_HOOK.x - PIVOT.x, LEFT_HOOK.y - PIVOT.y)
    const p = swing(LEFT_HOOK, PIVOT, 0.3)
    expect(Math.hypot(p.x - PIVOT.x, p.y - PIVOT.y)).toBeCloseTo(before, 9)
  })
})
