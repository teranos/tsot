import { describe, expect, test } from 'bun:test'
import { type TweakParam, tweakGlyph } from './tweak'

const PARAMS: TweakParam[] = [
  { key: 'zoom', label: 'zoom', group: 'shape', min: 0.2, max: 5, step: 0.01 },
  { key: 'folds', label: 'fractal folds', group: 'shape', min: 1, max: 6, step: 1 },
  { key: 'hue', label: 'hue', group: 'colour', min: 0, max: 1, step: 0.01 },
]

function setup() {
  const values: Record<string, number> = { zoom: 1, folds: 3, hue: 0 }
  const changes: Record<string, number>[] = []
  const errors: string[] = []
  const glyph = tweakGlyph({
    params: PARAMS,
    values,
    onChange: v => changes.push({ ...v }),
    onError: (where, e) => errors.push(`${where}: ${String(e)}`),
    onReset: () => {},
  })
  const panel = glyph.renderContent()
  const ranges = [...panel.querySelectorAll('input[type="range"]')] as HTMLInputElement[]
  const numbers = [...panel.querySelectorAll('input[type="number"]')] as HTMLInputElement[]
  return { values, changes, errors, glyph, panel, ranges, numbers }
}

function set(input: HTMLInputElement, value: string, event: 'input' | 'change') {
  input.value = value
  input.dispatchEvent(new Event(event))
}

describe('Tim: tweak is a glyph that opens as a window of tweaks', () => {
  test('it is the tweak glyph, and opens as a window', () => {
    const { glyph } = setup()
    expect(glyph.id).toBe('tweak')
    expect(glyph.title).toBe('tweak')
    expect(glyph.manifestationType).toBe('window')
  })

  test('one slider and one exact number box per tweak, at the current values', () => {
    const { ranges, numbers } = setup()
    expect(ranges.map(r => Number(r.value))).toEqual([1, 3, 0])
    expect(numbers.map(n => Number(n.value))).toEqual([1, 3, 0])
  })

  test('each slider carries its tweak range', () => {
    const { ranges } = setup()
    expect([ranges[1].min, ranges[1].max, ranges[1].step]).toEqual(['1', '6', '1'])
  })

  test('moving a slider changes the value and reports it', () => {
    const { values, changes, ranges, numbers } = setup()
    set(ranges[0], '2.5', 'input')
    expect(values.zoom).toBe(2.5)
    expect(numbers[0].value).toBe('2.5')
    expect(changes.at(-1)?.zoom).toBe(2.5)
  })

  test('tweaks are grouped under headings, in order', () => {
    const { panel } = setup()
    const headings = [...panel.querySelectorAll('.tweak-group')].map(h => h.textContent)
    expect(headings).toEqual(['shape', 'colour'])
  })
})

describe('Spike: values outside a tweak are pulled in or refused', () => {
  test('a number past the range is clamped to it', () => {
    const { values, numbers, ranges } = setup()
    set(numbers[2], '7', 'change')
    expect(values.hue).toBe(1)
    expect(ranges[2].value).toBe('1')
  })

  test('something that is not a number is refused, typed, and the value is kept', () => {
    const { values, numbers, errors } = setup()
    set(numbers[0], '', 'change')
    expect(values.zoom).toBe(1)
    expect(numbers[0].value).toBe('1')
    expect(errors.length).toBe(1)
  })
})

describe('Jenny: opening tweak again reveals what was always there', () => {
  test('the panel is the same element every time it is shown, never rebuilt', () => {
    const { glyph, panel } = setup()
    expect(glyph.renderContent()).toBe(panel)
  })

  test('a change made while it was closed is still there when it opens', () => {
    const { glyph, values, ranges } = setup()
    set(ranges[0], '4', 'input')
    const again = glyph.renderContent()
    const zoom = again.querySelector('input[type="range"]') as HTMLInputElement
    expect(zoom.value).toBe('4')
    expect(values.zoom).toBe(4)
  })
})
