export class BadRangeError extends Error {
  constructor(header: string, size: number, why: string) {
    super(`Range "${header}" on ${size} bytes: ${why}`)
    this.name = 'BadRangeError'
  }
}

export interface ByteRange {
  start: number
  end: number
}

// One `bytes=` range, inclusive ends. null header means the whole file.
export function byteRange(header: string | null, size: number): ByteRange | null {
  if (header === null) return null
  const bad = (why: string) => new BadRangeError(header, size, why)
  if (!header.startsWith('bytes=')) throw bad('only bytes ranges are served')
  const spec = header.slice('bytes='.length)
  if (spec.includes(',')) throw bad('multiple ranges are not served')
  const dash = spec.indexOf('-')
  if (dash === -1) throw bad('no "-" in range')
  const offset = (s: string): number | null => {
    const t = s.trim()
    if (t === '') return null
    const n = Number(t)
    if (!Number.isInteger(n) || n < 0) throw bad(`"${t}" is not a byte offset`)
    return n
  }
  const first = offset(spec.slice(0, dash))
  const last = offset(spec.slice(dash + 1))
  if (first === null) {
    if (last === null || last === 0) throw bad('empty range')
    return { start: Math.max(0, size - last), end: size - 1 }
  }
  if (first >= size) throw bad('start is past the end')
  if (last !== null && last < first) throw bad('end before start')
  return { start: first, end: last === null ? size - 1 : Math.min(last, size - 1) }
}
