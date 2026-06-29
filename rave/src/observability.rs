//! Rave's bridge from sacred-error typed errors to bevy-observability's
//! ErrorLog + to the HTML overlay via `__raveErrorTyped`.

#[cfg(target_arch = "wasm32")]
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use bevy_observability::{ErrorLog, Severity};

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
