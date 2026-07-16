//! The NPC-bump "!" overlay — Rust owns the render, drawn as UI quads
//! through the same overlay pipeline as the D-pad + HUD + watermark.
//!
//! On a bump, `trigger` stashes the NDC position under the "!". Each
//! frame, `age_and_publish` ages the state; when it ages past
//! BANG_TICKS the overlay clears. Re-triggering while active resets
//! the age so the "!" lingers as long as bumps keep landing.

use bevy_ecs::prelude::*;
use std::cell::RefCell;

use crate::dpad::DpadInstance;

/// ~60 frames/s × 1.2s tail.
const BANG_TICKS: u32 = 72;
/// One glyph pixel in CSS px. Bigger than the watermark (3px) so the
/// "!" reads above the NPC's head, not as a signature.
const PIXEL_PX: f32 = 6.0;
/// Warm yellow.
const COLOR: [f32; 3] = [0.98, 0.83, 0.13];

/// 3x5 pixel font for "!" — a vertical stroke, gap, dot.
const BANG_GLYPH: [u8; 5] = [0b010, 0b010, 0b010, 0b000, 0b010];

#[derive(Default)]
pub struct BangState {
    pub ndc: [f32; 2],
    pub age: u32,
}

#[derive(Resource, Default)]
pub struct Bang {
    pub active: Option<BangState>,
}

pub fn setup_bang(mut commands: Commands) {
    commands.insert_resource(Bang::default());
}

/// Called from `physics::check_npc_bump` when the player overlaps an
/// NPC. `ndc` is the "!" anchor in clip-space (== NDC for our ortho
/// camera), computed above the NPC's head.
pub fn trigger(bang: &mut Bang, ndc: [f32; 2]) {
    bang.active = Some(BangState { ndc, age: 0 });
}

thread_local! {
    static BANG_INSTANCES: RefCell<Vec<DpadInstance>> =
        const { RefCell::new(Vec::new()) };
}

/// The bang quads for the current frame — appended after the D-pad,
/// HUD, and watermark by render_web.
pub fn current_instances() -> Vec<DpadInstance> {
    BANG_INSTANCES.with(|c| c.borrow().clone())
}

/// Per-tick: age the active bang, drop it after BANG_TICKS, publish
/// the "!" glyph quads at its NDC position.
pub fn age_and_publish(mut bang: ResMut<Bang>) {
    let publish = if let Some(state) = bang.active.as_mut() {
        state.age = state.age.wrapping_add(1);
        if state.age > BANG_TICKS {
            bang.active = None;
            None
        } else {
            Some((state.ndc, state.age))
        }
    } else {
        None
    };
    let Some((ndc, _age)) = publish else {
        BANG_INSTANCES.with(|c| c.borrow_mut().clear());
        return;
    };
    let quads = bang_quads(ndc, crate::gpu_web::viewport_size());
    BANG_INSTANCES.with(|c| *c.borrow_mut() = quads);
}

/// Pure quad-emitter — glyph pixels at `ndc`, one CSS-px per font
/// pixel, sized in NDC via the current viewport. Sits above `ndc`
/// so the "!" appears over the NPC's head, not on it.
pub fn bang_quads(ndc: [f32; 2], viewport: (u32, u32)) -> Vec<DpadInstance> {
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    let sx = PIXEL_PX * (2.0 / w);
    let sy = PIXEL_PX * (2.0 / h);
    let cols = 3usize;
    let rows = BANG_GLYPH.len();
    // Centre the glyph horizontally on ndc.x; sit its base a glyph
    // height above ndc.y (so it floats above the NPC).
    let left = ndc[0] - (cols as f32 * sx) * 0.5;
    let top = ndc[1] + rows as f32 * sy;
    let mut out = Vec::with_capacity(rows * cols);
    for (r, row) in BANG_GLYPH.iter().enumerate() {
        for c in 0..cols {
            if row & (1 << (cols - 1 - c)) != 0 {
                out.push(DpadInstance {
                    center_ndc: [
                        left + (c as f32 + 0.5) * sx,
                        top - (r as f32 + 0.5) * sy,
                    ],
                    half_size_ndc: [sx * 0.5, sy * 0.5],
                    color: COLOR,
                    alpha: 1.0,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_stashes_position_and_resets_age() {
        let mut b = Bang::default();
        trigger(&mut b, [0.5, -0.3]);
        let s = b.active.as_ref().unwrap();
        assert_eq!(s.ndc, [0.5, -0.3]);
        assert_eq!(s.age, 0);
        // Re-trigger resets the age.
        b.active.as_mut().unwrap().age = 40;
        trigger(&mut b, [0.6, -0.2]);
        assert_eq!(b.active.as_ref().unwrap().age, 0);
    }

    #[test]
    fn bang_quads_sit_above_the_anchor_and_hit_four_dots() {
        // The 3x5 "!" glyph lights 4 pixels: a 3-tall stem + one dot.
        let quads = bang_quads([0.0, 0.0], (1920, 1080));
        assert_eq!(quads.len(), 4);
        for q in &quads {
            assert!(q.center_ndc[1] > 0.0, "bang floats above anchor");
        }
    }

    #[test]
    fn age_and_publish_expires_after_bang_ticks() {
        // Aging past the budget drops the resource and clears quads.
        // Runs enough system frames that the age exceeds BANG_TICKS.
        use bevy_ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Bang::default());
        {
            let mut b = world.resource_mut::<Bang>();
            trigger(&mut b, [0.0, 0.0]);
        }
        for _ in 0..(BANG_TICKS + 2) {
            world.run_system_once(age_and_publish).unwrap();
        }
        assert!(world.resource::<Bang>().active.is_none());
    }
}
