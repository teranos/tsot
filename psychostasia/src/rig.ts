// The brass scale seen from the front: x runs left to right, y is up.
export interface Point {
  x: number
  y: number
}

// The scale turns about its column, away from the cursor: cursor left of
// centre turns it right, right of centre turns it left. Vertical is ignored.
export const MAX_YAW = (30 * Math.PI) / 180

export function yaw(cursorX: number, width: number, max: number = MAX_YAW): number {
  const off = Math.min(1, Math.max(-1, (cursorX / width) * 2 - 1))
  return (0 - off) * max
}

// Rotate a point about the beam's pivot. A positive angle raises the
// left side (x < pivot.x), where the heart hangs.
export function swing(p: Point, pivot: Point, angle: number): Point {
  const dx = p.x - pivot.x
  const dy = p.y - pivot.y
  const c = Math.cos(angle)
  const s = Math.sin(angle)
  return { x: pivot.x + dx * c + dy * s, y: pivot.y - dx * s + dy * c }
}
