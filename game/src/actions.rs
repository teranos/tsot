//! The three action slots.
//!
//! At any moment there are exactly three things you can do. They live
//! in the bottom-left corner, they are tappable, and on desktop they
//! are the same three keys forever. What they DO is contextual; where
//! they are never changes.
//!
//! This is a limitation, deliberately, and it is load-bearing rather
//! than cosmetic. It means:
//!
//! - Every verb the game grows has to earn a slot from an existing one.
//!   There is no room to accumulate a keyboard nobody can remember.
//! - Touch and desktop are the same game. Three buttons is a thumb's
//!   reach; three keys is a hand that never leaves WASD.
//! - Discoverability is structural, not documentation. The available
//!   verb is on screen because it is available. Nobody has to be told
//!   that E means become — which is exactly how that went.
//!
//! The bar is a PROJECTION of world state, recomputed every frame from
//! `Affordances`, never a stored menu. A stored one goes stale and
//! offers "chop" after you have walked away from the tree.

/// Three. Not "up to three".
pub const SLOTS: usize = 3;

/// Everything the world affords the human this instant. Deliberately a
/// tiny plain struct rather than ECS queries: the resolver is then a
/// pure function that a test can drive through every context in
/// microseconds, and the ECS side is one system that fills this in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Affordances {
    /// How many commandable units are selected.
    pub selected_count: usize,
    /// Whether the human is currently driving a body.
    pub piloting: bool,
}

/// A verb the human can invoke. Each owns a fixed slot — a verb is not
/// "whatever is free", it is a thing your hand learns the position of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Take the wheel of the one selected body.
    Become,
    /// Step back out into the detached observer.
    Leave,
}

impl Action {
    /// The slot this verb always occupies. Fixed per verb, so the same
    /// action never moves under the hand between contexts.
    ///
    /// `Become` and `Leave` share slot 1: they are one verb pointing
    /// opposite ways, so stepping in and out of a body is the same key
    /// twice, not two keys to remember.
    pub fn slot(self) -> usize {
        match self {
            Action::Become | Action::Leave => 1,
        }
    }

    /// Who wins when two verbs want the same slot. Higher takes it; the
    /// loser is simply not offered.
    ///
    /// Dropping rather than growing is the constraint doing its work.
    /// When a context wants a fourth verb, someone has to decide what it
    /// displaces — the alternative is a bar that quietly becomes four
    /// buttons and then five.
    pub fn priority(self) -> u8 {
        match self {
            // Getting out of a body beats getting into one: you are in
            // it, so the way out is the more urgent offer.
            Action::Leave => 20,
            Action::Become => 10,
        }
    }

    /// Drawn with the 3x5 pixel font in `watermark`, which knows
    /// lowercase, digits and a little punctuation — so labels are
    /// lowercase words, short enough to fit a button.
    pub fn label(self) -> &'static str {
        match self {
            Action::Become => "become",
            Action::Leave => "leave",
        }
    }
}

/// Which verbs the world is offering right now, by slot.
///
/// Every candidate is generated, then each claims its declared slot,
/// highest priority winning. Nothing is shuffled into a free slot, and
/// nothing is offered that did not claim one.
pub fn resolve(a: Affordances) -> [Option<Action>; SLOTS] {
    let mut bar: [Option<Action>; SLOTS] = [None; SLOTS];
    for candidate in candidates(a) {
        let slot = candidate.slot();
        debug_assert!(slot < SLOTS, "{candidate:?} declares slot {slot}, out of range");
        let take = match bar[slot] {
            None => true,
            Some(held) => candidate.priority() > held.priority(),
        };
        if take {
            bar[slot] = Some(candidate);
        }
    }
    bar
}

/// Every verb whose precondition currently holds. Order does not
/// matter — `resolve` places by slot and priority, not by arrival.
fn candidates(a: Affordances) -> Vec<Action> {
    let mut out = Vec::new();
    if a.piloting {
        // In a body: the way out. Becoming is meaningless here, and
        // offering it would imply you could be in two at once.
        out.push(Action::Leave);
    } else if a.selected_count == 1 {
        // Exactly one, so the verb is never ambiguous about which body
        // it means. Becoming a squad is not a thing.
        out.push(Action::Become);
    }
    out
}

// --- Where the three slots live on screen ---

use crate::dpad::DpadInstance;
use crate::hud::Rect;

/// Button edge and spacing, in CSS pixels. Sized for a thumb: the
/// buttons are the touch control and the keys are the shortcut, not the
/// other way round.
///
/// 88 clears Apple's 44pt minimum touch target twice over, because
/// these are the primary control on a phone and they are pressed while
/// the other thumb is on the D-pad — the hand is not steady.
pub const BUTTON_PX: f32 = 88.0;
pub const MARGIN_PX: f32 = 20.0;
pub const GAP_PX: f32 = 10.0;
/// Label pixel scale for the 3x5 font. "become" is 6 chars → 6·4·3 =
/// 72px, inside an 88px button with room either side.
const LABEL_PIXEL_PX: f32 = 3.0;
/// The key hint drawn on each button, in slot order. The button and the
/// key are the same control, so the button says which key it is.
pub const SLOT_KEYS: [&str; SLOTS] = ["q", "e", "r"];

/// The three button rectangles, bottom-left, left to right.
///
/// Computed from the viewport every frame and never cached: the row
/// must not move when the window resizes, and it must never reflow
/// when a slot is empty — an empty slot keeps its rectangle, because a
/// row that shrinks moves the buttons beside it and thumb memory dies.
pub fn slot_rects(viewport: (u32, u32)) -> [Rect; SLOTS] {
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    let (ndc_x, ndc_y) = (2.0 / w, 2.0 / h);
    let (bw, bh) = (BUTTON_PX * ndc_x, BUTTON_PX * ndc_y);
    let cy = -1.0 + MARGIN_PX * ndc_y + bh * 0.5;
    std::array::from_fn(|i| Rect {
        cx: -1.0 + MARGIN_PX * ndc_x + bw * 0.5 + i as f32 * (bw + GAP_PX * ndc_x),
        cy,
        hx: bw * 0.5,
        hy: bh * 0.5,
    })
}

/// Overlay quads for the bar: three buttons, each with its key hint,
/// and a label on the ones that are offering something.
///
/// Empty slots are drawn dim rather than omitted. Seeing that there are
/// three, and that two of them have nothing right now, is the point —
/// it tells you the shape of the game's whole verb surface at a glance.
pub fn build_quads(bar: &[Option<Action>; SLOTS], viewport: (u32, u32)) -> Vec<DpadInstance> {
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    let (ndc_x, ndc_y) = (2.0 / w, 2.0 / h);
    let (sx, sy) = (LABEL_PIXEL_PX * ndc_x, LABEL_PIXEL_PX * ndc_y);
    let mut out = Vec::with_capacity(SLOTS * 24);

    for (i, rect) in slot_rects(viewport).iter().enumerate() {
        let filled = bar[i].is_some();
        out.push(DpadInstance {
            center_ndc: [rect.cx, rect.cy],
            half_size_ndc: [rect.hx, rect.hy],
            color: if filled { [0.16, 0.30, 0.22] } else { [0.10, 0.10, 0.13] },
            alpha: if filled { 0.92 } else { 0.55 },
        });

        // Key hint, top-left inside the button. Always drawn, even on
        // an empty slot: the binding is a fact about the slot, not
        // about whatever it happens to offer.
        let pad_x = 5.0 * ndc_x;
        let pad_y = 5.0 * ndc_y;
        out.extend(crate::watermark::text_quads(
            SLOT_KEYS[i],
            rect.cx - rect.hx + pad_x,
            rect.cy + rect.hy - pad_y,
            [sx, sy],
            if filled { [0.75, 0.90, 0.78] } else { [0.38, 0.38, 0.44] },
            if filled { 0.95 } else { 0.6 },
        ));

        if let Some(action) = bar[i] {
            let label = action.label();
            let width = label.chars().count() as f32 * (GLYPH_ADVANCE as f32) * sx;
            out.extend(crate::watermark::text_quads(
                label,
                rect.cx - width * 0.5,
                rect.cy - 2.0 * ndc_y,
                [sx, sy],
                [0.85, 1.0, 0.88],
                1.0,
            ));
        }
    }
    out
}

/// Glyph cell width including the one-pixel gap `text_quads` inserts.
const GLYPH_ADVANCE: usize = crate::watermark::GLYPH_W + 1;

/// Slot key bits for every touch currently inside an action button.
///
/// A tap becomes the SAME key bit the keyboard produces, so the button
/// and the key are genuinely one control: one edge detector, one
/// dispatch, one place where a verb can be invoked. A separate touch
/// path would be a second way to invoke actions, and the two would
/// drift.
///
/// Pure, and takes its touches as an argument, so it can be tested
/// without a browser — the wasm side is one call, not a branch full of
/// logic no host-target test can see.
pub fn slots_touched(viewport: (u32, u32), touches: &[[f32; 2]]) -> u32 {
    let rects = slot_rects(viewport);
    let mut bits = 0;
    for t in touches {
        for (i, r) in rects.iter().enumerate() {
            if r.contains(*t) {
                bits |= crate::input::key::slot_bit(i);
            }
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_priority_verb_takes_a_contested_slot() {
        // No context produces a contest today — `candidates` never
        // emits Become and Leave together. This pins the RULE anyway,
        // because the first verb pair that does contend will be added
        // by someone who needs it to already work.
        let mut bar: [Option<Action>; SLOTS] = [None; SLOTS];
        for c in [Action::Become, Action::Leave] {
            let slot = c.slot();
            if bar[slot].is_none_or(|held| c.priority() > held.priority()) {
                bar[slot] = Some(c);
            }
        }
        assert_eq!(bar[1], Some(Action::Leave), "the higher priority verb lost its slot");
    }

    #[test]
    fn every_verb_declares_a_slot_in_range() {
        for a in [Action::Become, Action::Leave] {
            assert!(a.slot() < SLOTS, "{a:?} declares slot {} of {SLOTS}", a.slot());
        }
    }
}
