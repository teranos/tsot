//! Bevy adapter for `laye-input`.
//!
//! - [`InputCapturePlugin`] (B4): inserts an [`InputCapture`] resource
//!   that wraps `laye_input::InputClaims`. UI plugins claim/release on
//!   focus/blur; world systems gate on `is_captured()`.
//! - [`DefaultBindingsPlugin`] (B5): reads the keyboard each frame and
//!   emits [`IntentEvent`]s for the conventional defaults (Esc, T,
//!   `` ` ``/`\`, P, Tab). Also handles `Intent::ReleaseAll` by
//!   clearing the capture set, so Esc is the universal "back to
//!   world" gesture.

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_input::prelude::*;

pub use laye_input::Intent;

/// Bevy resource wrapping the engine-agnostic [`laye_input::InputClaims`].
/// Newtype so the Resource derive + orphan rule are satisfied without
/// polluting `laye-input` with a Bevy dependency.
#[derive(Resource, Default, Debug)]
pub struct InputCapture(pub laye_input::InputClaims);

impl InputCapture {
    pub fn claim(&mut self, who: &'static str) {
        self.0.claim(who);
    }
    pub fn release(&mut self, who: &'static str) {
        self.0.release(who);
    }
    pub fn release_all(&mut self) {
        self.0.release_all();
    }
    pub fn is_captured(&self) -> bool {
        self.0.is_captured()
    }
    pub fn claimants(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.claimants()
    }
}

/// B4 — arbitration only. Inserts `InputCapture`; the consumer wires
/// claim/release into its own UI focus events.
pub struct InputCapturePlugin;

impl Plugin for InputCapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(InputCapture::default());
    }
}

/// Event emitted when a bound key fires. Consumers add a
/// `MessageReader<IntentEvent>` and match on `Intent`.
#[derive(Message, Debug, Clone, Copy)]
pub struct IntentEvent(pub Intent);

/// B5 — default key bindings. Reads keyboard, emits [`IntentEvent`]s
/// for the conventional defaults. Auto-handles `Intent::ReleaseAll`
/// by clearing the capture set.
pub struct DefaultBindingsPlugin;

impl Plugin for DefaultBindingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<IntentEvent>();
        app.add_systems(Update, (emit_default_intents, handle_release_all).chain());
    }
}

fn emit_default_intents(
    keys: Res<ButtonInput<KeyCode>>,
    mut writer: MessageWriter<IntentEvent>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        writer.write(IntentEvent(Intent::ReleaseAll));
    }
    if keys.just_pressed(KeyCode::KeyT) {
        writer.write(IntentEvent(Intent::ChatFocus));
    }
    if keys.just_pressed(KeyCode::Backquote) || keys.just_pressed(KeyCode::Backslash) {
        writer.write(IntentEvent(Intent::DrawerToggle));
    }
    if keys.just_pressed(KeyCode::KeyP) {
        writer.write(IntentEvent(Intent::Screenshot));
    }
    if keys.just_pressed(KeyCode::Tab) {
        writer.write(IntentEvent(Intent::InventoryToggle));
    }
}

fn handle_release_all(
    mut reader: MessageReader<IntentEvent>,
    mut cap: ResMut<InputCapture>,
) {
    for IntentEvent(intent) in reader.read() {
        if *intent == Intent::ReleaseAll {
            cap.release_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_capture_plugin_inserts_resource() {
        let mut app = App::new();
        app.add_plugins(InputCapturePlugin);
        let cap = app.world().get_resource::<InputCapture>();
        assert!(cap.is_some(), "plugin must insert InputCapture");
        assert!(!cap.expect("just asserted Some").is_captured());
    }

    #[test]
    fn input_capture_methods_delegate_to_laye_input() {
        let mut cap = InputCapture::default();
        cap.claim("chat");
        assert!(cap.is_captured());
        cap.release("chat");
        assert!(!cap.is_captured());
    }

    #[test]
    fn release_all_clears_capture_set() {
        let mut cap = InputCapture::default();
        cap.claim("chat");
        cap.claim("wardrobe");
        cap.release_all();
        assert!(!cap.is_captured());
    }
}
