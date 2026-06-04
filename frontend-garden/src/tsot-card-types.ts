// CardView — TypeScript mirror of `src/sim/snapshot.rs::CardView` in
// the tsot engine. The engine serializes this shape via serde-JSON;
// the UI consumes it.
//
// Hand-typed for now. When the type set stabilizes, wire `ts-rs` on
// the Rust side and replace this file with generated `.d.ts`.

export interface CardView {
  iid: string;
  id: string;
  name: string;
  kind: string;
  colors: string[];
  /**
   * Symbols printed on the back of the card (C.1, SLOTS.md). The
   * engine emits a flat array; UI applies the SLOTS.md spiral default
   * fill (`C, U, UR, R, DR, D, DL, L, UL, TL, T, TR, BR, B, BL`) to
   * assign each glyph to a slot. When the engine surfaces per-slot
   * placement explicitly, this becomes a Record<Slot, string>.
   */
  symbols: string[];
  subtypes: string[];
  /** Printed cost as written on the card. */
  cost: string;
  /** Cost after static reductions; equals `cost` outside HAND. */
  effective_cost: string;
  abilities: string[];
  flavor: string;
  tapped: boolean;
  summoning_sick: boolean;
  damage: number;
  power: number;
  toughness: number;
  attached: CardView[];
  /**
   * Frame attribute (C.13). `'transparent'` means every slot is a
   * hole until per-slot data lands. Not yet emitted by the engine —
   * mockup-only for now.
   */
  frame?: 'transparent' | string;
}

export type CardFace = 'back' | 'front';
