import type { Glyph } from '@jsr/qntx__glyphs'

// tweak: every adjustable number of a piece, as a glyph. It rests in the tray
// as a dot and opens as a window of tweaks. The panel is built once and the
// same element is shown every time it opens: nothing is reconstructed.

export interface TweakParam {
  key: string
  label: string
  group: string
  min: number
  max: number
  step: number
}

export interface TweakOptions {
  params: readonly TweakParam[]
  // Changed in place, so the host keeps reading the one object it owns.
  values: Record<string, number>
  onChange: (values: Record<string, number>) => void
  onError: (where: string, e: unknown) => void
  onReset: () => void
}

export function tweakGlyph(o: TweakOptions): Glyph {
  let panel: HTMLElement | null = null
  return {
    id: 'tweak',
    title: 'tweak',
    symbol: '≈',
    manifestationType: 'window',
    initialWidth: '460px',
    initialHeight: '620px',
    renderContent: () => {
      panel ??= build(o)
      return panel
    },
  }
}

function build(o: TweakOptions): HTMLElement {
  const panel = document.createElement('div')
  panel.className = 'tweak'
  const json = document.createElement('pre')
  json.className = 'tweak-json'
  const showJson = () => {
    json.textContent = JSON.stringify(o.values, null, 1)
  }

  let group = ''
  for (const p of o.params) {
    if (p.group !== group) {
      group = p.group
      const heading = document.createElement('div')
      heading.className = 'tweak-group'
      heading.textContent = group
      panel.append(heading)
    }
    const row = document.createElement('label')
    row.className = 'tweak-row'
    const name = document.createElement('span')
    name.textContent = p.label
    const range = document.createElement('input')
    const num = document.createElement('input')
    range.type = 'range'
    num.type = 'number'
    for (const input of [range, num]) {
      input.min = String(p.min)
      input.max = String(p.max)
      input.step = String(p.step)
      input.value = String(o.values[p.key])
    }
    const set = (v: number) => {
      o.values[p.key] = Math.min(p.max, Math.max(p.min, v))
      range.value = num.value = String(o.values[p.key])
      showJson()
      o.onChange(o.values)
    }
    range.addEventListener('input', () => set(Number(range.value)))
    num.addEventListener('change', () => {
      const v = Number(num.value)
      if (num.value.trim() === '' || !Number.isFinite(v)) {
        o.onError(`tweak: "${num.value}" is not a number for ${p.label}`, `kept ${o.values[p.key]}`)
        num.value = String(o.values[p.key])
        return
      }
      set(v)
    })
    row.append(name, range, num)
    panel.append(row)
  }

  const actions = document.createElement('div')
  actions.className = 'tweak-actions'
  const copy = document.createElement('button')
  copy.textContent = 'copy'
  copy.addEventListener('click', () =>
    navigator.clipboard.writeText(JSON.stringify(o.values, null, 1)).catch(e => o.onError('tweak: copy to clipboard failed', e)),
  )
  const reset = document.createElement('button')
  reset.textContent = 'reset'
  reset.addEventListener('click', () => o.onReset())
  actions.append(copy, reset)
  panel.append(actions, json)
  showJson()
  return panel
}
