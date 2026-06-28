//! Panic + tracing + typed-error capture pipeline.
//!
//! Three sinks feed `ErrorLog`:
//!   - the panic hook (via the static `PANIC_QUEUE`)
//!   - Bevy's tracing layer (via the static `LOG_QUEUE`, installed
//!     into `LogPlugin.custom_layer` from `lib.rs::run`)
//!   - the typed `sacred_error::Error` pipeline drained from
//!     `crate::error` on wasm32
//!
//! Drainers run in `Update`. The drawer reads `ErrorLog` and renders
//! the lines. Sacred-error path crossing into JS lives here too so
//! every wasm→JS boundary the panic hook + tracer reach goes through
//! the same module.

use bevy::log::{
    tracing::{self, Subscriber},
    tracing_subscriber::Layer,
    BoxedLayer,
};
use bevy::prelude::*;
use std::sync::Mutex;

/// Queue from the panic hook into the ECS. The hook runs outside Bevy
/// systems, so it can't write `ErrorLog` directly. Drained each frame
/// by [`drain_panics`].
pub static PANIC_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Queue from Bevy's tracing layer (info!/warn!/error! and Bevy's own
/// emits) into the ECS. Drained each frame by [`drain_logs`].
pub static LOG_QUEUE: Mutex<Vec<(Severity, String)>> = Mutex::new(Vec::new());

/// Cap to last N entries so a runaway producer (gossip-flood, panic
/// loop) can't grow the in-canvas text node unbounded and tank FPS.
pub const ERROR_LOG_CAP: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Note,
    Warn,
    Error,
}

pub struct ErrorEntry {
    pub severity: Severity,
    pub message: String,
}

#[derive(Resource, Default)]
pub struct ErrorLog(pub Vec<ErrorEntry>);

impl ErrorLog {
    pub fn emit(&mut self, severity: Severity, message: impl Into<String>) {
        self.0.push(ErrorEntry {
            severity,
            message: message.into(),
        });
        if self.0.len() > ERROR_LOG_CAP {
            let drop = self.0.len() - ERROR_LOG_CAP;
            self.0.drain(0..drop);
        }
    }
}

/// LogPlugin's `custom_layer` hook — called once at plugin build time.
/// Pattern from
/// https://github.com/bevyengine/bevy/blob/v0.19.0/examples/app/log_layers.rs
pub fn install_capture_layer(_app: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(CaptureLayer))
}

/// Captures every tracing event Bevy or our code emits. Only WARN +
/// ERROR levels propagate to the in-canvas drawer; INFO/DEBUG/TRACE
/// would flood it. LogPlugin's default fmt layer still emits everything
/// to the browser console, so the lower-severity events are not lost —
/// they live in the console channel.
pub struct CaptureLayer;

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: bevy::log::tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = *event.metadata().level();
        let severity = match level {
            tracing::Level::ERROR => Severity::Error,
            tracing::Level::WARN => Severity::Warn,
            _ => return,
        };

        let target = event.metadata().target();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let formatted = format!("{target}: {}", visitor.message);

        // ERROR-level tracing skips the drawer queue and goes straight
        // to the HTML overlay too — survives the Bevy-never-runs case.
        if matches!(severity, Severity::Error) {
            crate::js_rave_error(&format!("[tracing ERROR] {formatted}"));
        }

        let mut q = LOG_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
        q.push((severity, formatted));
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

pub fn drain_panics(mut log: ResMut<ErrorLog>) {
    let mut q = PANIC_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    for msg in q.drain(..) {
        log.emit(Severity::Error, format!("PANIC: {msg}"));
    }
}

pub fn drain_logs(mut log: ResMut<ErrorLog>) {
    let mut q = LOG_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    for (sev, msg) in q.drain(..) {
        log.emit(sev, msg);
    }
}

/// Pulls every typed Error from the `crate::error` thread_local buffer,
/// pushes each to the in-canvas drawer (formatted with
/// severity/region/title) AND to the HTML overlay via
/// `__raveErrorTyped` (typed JSON, so the receiving JS keeps the
/// structured fields). Single source of truth for typed errors
/// crossing the wasm→JS boundary.
#[cfg(target_arch = "wasm32")]
pub fn flush_typed_errors(mut error_log: ResMut<ErrorLog>) {
    for err in crate::error::drain() {
        let region = err.context.region.as_deref().unwrap_or("?");
        let severity_for_drawer = match err.severity {
            sacred_error::Severity::Info => Severity::Note,
            sacred_error::Severity::Warn => Severity::Warn,
            sacred_error::Severity::Error => Severity::Error,
            sacred_error::Severity::Panic => Severity::Error,
        };
        error_log.emit(
            severity_for_drawer,
            format!("[{region}] {} — {}", err.title, err.why),
        );
        match serde_json::to_string(&err) {
            Ok(json) => crate::js_rave_error_typed(&json),
            Err(e) => crate::js_rave_error(&format!("[flush_typed_errors serialize] {e}")),
        }
    }
}
