import { describe, expect, test } from 'bun:test'
import { BadRangeError, byteRange } from './range'

const SIZE = 1000

describe('byteRange: video needs byte ranges to seek and loop', () => {
  test('no Range header serves the whole file', () => {
    expect(byteRange(null, SIZE)).toBeNull()
  })

  test('a closed range', () => {
    expect(byteRange('bytes=0-99', SIZE)).toEqual({ start: 0, end: 99 })
  })

  test('an open range runs to the end', () => {
    expect(byteRange('bytes=500-', SIZE)).toEqual({ start: 500, end: 999 })
  })

  test('a suffix range is the last n bytes', () => {
    expect(byteRange('bytes=-100', SIZE)).toEqual({ start: 900, end: 999 })
  })

  test('an end past the file is clamped', () => {
    expect(byteRange('bytes=900-5000', SIZE)).toEqual({ start: 900, end: 999 })
  })

  test('a start past the file is refused, typed', () => {
    expect(() => byteRange('bytes=1000-', SIZE)).toThrow(BadRangeError)
  })

  test('anything but one bytes range is refused, typed', () => {
    expect(() => byteRange('items=0-1', SIZE)).toThrow(BadRangeError)
    expect(() => byteRange('bytes=abc-', SIZE)).toThrow(BadRangeError)
    expect(() => byteRange('bytes=0-1,5-6', SIZE)).toThrow(BadRangeError)
    expect(() => byteRange('bytes=50-10', SIZE)).toThrow(BadRangeError)
  })
})
