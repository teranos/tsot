//! Engine-agnostic input concerns. No Bevy here — that lives in
//! `bevy-input-capture`. Two primitives:
//!
//! - [`InputClaims`] — a set of named claimants. UI elements claim
//!   when they want exclusive input; world systems gate on
//!   `is_captured()`. Engine-agnostic data, zero deps.
//! - [`Intent`] — the abstract vocabulary of user-input meanings
//!   (focus chat, toggle drawer, take screenshot, …). Engine-agnostic.
//!   Engine adapters bind their own concrete keys/buttons to these.

use std::collections::HashSet;

/// Set of currently-active claimants. Non-empty ⇒ some UI element is
/// consuming input; world systems should skip their keystroke
/// handling. Keys are `&'static str` so consumers from different
/// crates can claim without coordinating an enum.
#[derive(Default, Debug, Clone)]
pub struct InputClaims(HashSet<&'static str>);

impl InputClaims {
    /// Mark `who` as actively consuming input. Idempotent.
    pub fn claim(&mut self, who: &'static str) {
        self.0.insert(who);
    }

    /// Remove `who` from the claim set. No-op if not present.
    pub fn release(&mut self, who: &'static str) {
        self.0.remove(who);
    }

    /// Drop every active claim. Maps to a global "Esc → return to
    /// world" gesture; engine adapters call this on `Intent::ReleaseAll`.
    pub fn release_all(&mut self) {
        self.0.clear();
    }

    /// True when at least one claimant is active.
    pub fn is_captured(&self) -> bool {
        !self.0.is_empty()
    }

    /// Iterate currently active claim keys. Useful for diagnostics
    /// ("who is holding input right now?").
    pub fn claimants(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().copied()
    }
}

/// Abstract user-input meanings. Engine adapters bind concrete keys
/// or buttons to these and emit an `Intent` (often wrapped in an
/// engine-specific event type) when the bound input fires. Consumers
/// match on `Intent` rather than on key codes, so the same game logic
/// works across engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Request that the chat input gain focus. The chat UI handles
    /// the actual focus + `claim("chat")`.
    ChatFocus,
    /// Request to toggle the diagnostic drawer overlay.
    DrawerToggle,
    /// Take a screenshot of the current frame.
    Screenshot,
    /// Toggle inventory / extended UI panel.
    InventoryToggle,
    /// Release every active input claim. Mapped to Esc by convention.
    ReleaseAll,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_not_captured() {
        let c = InputClaims::default();
        assert!(!c.is_captured());
        assert_eq!(c.claimants().count(), 0);
    }

    #[test]
    fn claim_marks_captured() {
        let mut c = InputClaims::default();
        c.claim("chat");
        assert!(c.is_captured());
        assert!(c.claimants().any(|k| k == "chat"));
    }

    #[test]
    fn release_removes_claimant() {
        let mut c = InputClaims::default();
        c.claim("chat");
        c.release("chat");
        assert!(!c.is_captured());
    }

    #[test]
    fn release_unknown_is_noop() {
        let mut c = InputClaims::default();
        c.release("nobody");
        assert!(!c.is_captured());
    }

    #[test]
    fn claim_is_idempotent() {
        let mut c = InputClaims::default();
        c.claim("chat");
        c.claim("chat");
        c.release("chat");
        assert!(!c.is_captured());
    }

    #[test]
    fn multiple_claimants_compose() {
        let mut c = InputClaims::default();
        c.claim("chat");
        c.claim("wardrobe");
        c.release("chat");
        assert!(c.is_captured(), "wardrobe still holding");
        c.release("wardrobe");
        assert!(!c.is_captured());
    }

    #[test]
    fn release_all_clears_every_claim() {
        let mut c = InputClaims::default();
        c.claim("chat");
        c.claim("wardrobe");
        c.claim("obelisk-plaque");
        c.release_all();
        assert!(!c.is_captured());
        assert_eq!(c.claimants().count(), 0);
    }

    #[test]
    fn intent_is_copy_and_hashable() {
        let a = Intent::ChatFocus;
        let b = a;
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(Intent::Screenshot);
        set.insert(Intent::Screenshot);
        assert_eq!(set.len(), 1);
    }
}
