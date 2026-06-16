#![allow(dead_code)]

use std::cell::RefCell;

use crate::trace::{drain_json, emit, pending_count, TraceEvent};
use crate::world::World;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

thread_local! {
    static WORLD: RefCell<Option<World>> = const { RefCell::new(None) };
}

pub(crate) fn roam_init_impl() {
    WORLD.with(|w| *w.borrow_mut() = Some(World::new()));
}

pub(crate) fn roam_tick_impl(input: u32, dt_ms: f32) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.step(input, dt_ms);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_tick",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

pub(crate) fn roam_state_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::state_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_state",
                    msg: "called before roam_init; returning {}".to_string(),
                });
                "{}".to_string()
            })
    })
}

pub(crate) fn roam_map_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::map_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_map",
                    msg: "called before roam_init; returning {}".to_string(),
                });
                "{}".to_string()
            })
    })
}

pub(crate) fn roam_set_position_impl(x: f32, y: f32, facing: u8) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.set_position(x, y, facing);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_set_position",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

pub(crate) fn roam_viewport_impl(view_w: u32, view_h: u32) -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(|world| world.viewport_json(view_w, view_h))
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_viewport",
                    msg: "called before roam_init; returning {}".to_string(),
                });
                "{}".to_string()
            })
    })
}

pub(crate) fn roam_drain_trace_impl() -> String {
    drain_json()
}

pub(crate) fn roam_trace_pending_count_impl() -> u32 {
    pending_count() as u32
}

pub(crate) fn roam_drain_errors_impl() -> String {
    let errors = crate::error::drain();
    serde_json::to_string(&errors).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) fn roam_session_snapshot_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::session_snapshot_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_session_snapshot",
                    msg: "called before roam_init; returning empty".to_string(),
                });
                r#"{"picked":[],"inv":[0,0,0,0,0,0,0]}"#.to_string()
            })
    })
}

pub(crate) fn roam_restore_session_impl(raw: String) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.restore_session_json(&raw);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_restore_session",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use super::*;

    #[wasm_bindgen]
    pub fn roam_init() {
        super::roam_init_impl();
    }

    #[wasm_bindgen]
    pub fn roam_tick(input: u32, dt_ms: f32) {
        super::roam_tick_impl(input, dt_ms);
    }

    #[wasm_bindgen]
    pub fn roam_state() -> String {
        super::roam_state_impl()
    }

    #[wasm_bindgen]
    pub fn roam_map() -> String {
        super::roam_map_impl()
    }

    #[wasm_bindgen]
    pub fn roam_viewport(view_w: u32, view_h: u32) -> String {
        super::roam_viewport_impl(view_w, view_h)
    }

    #[wasm_bindgen]
    pub fn roam_set_position(x: f32, y: f32, facing: u8) {
        super::roam_set_position_impl(x, y, facing);
    }

    #[wasm_bindgen]
    pub fn roam_drain_trace() -> String {
        super::roam_drain_trace_impl()
    }

    #[wasm_bindgen]
    pub fn roam_trace_pending_count() -> u32 {
        super::roam_trace_pending_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_session_snapshot() -> String {
        super::roam_session_snapshot_impl()
    }

    #[wasm_bindgen]
    pub fn roam_restore_session(raw: String) {
        super::roam_restore_session_impl(raw);
    }

    #[wasm_bindgen]
    pub fn roam_drain_errors() -> String {
        super::roam_drain_errors_impl()
    }
}
