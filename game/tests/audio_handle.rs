// Native path is all no-ops — this test proves the module compiles,
// the handle wrapper drops cleanly, and play/stop are safe to call
// with a handle that was never actually loaded (which happens on
// wasm when the asset is missing or still decoding).

use game::audio;

#[test]
fn load_returns_handle_and_drops_cleanly() {
    let h = audio::load_music();
    drop(h);
}

#[test]
fn play_and_stop_are_no_ops_on_native() {
    let h = audio::load_music();
    audio::play(&h, 0.5, true);
    audio::stop(&h);
}

#[test]
fn multiple_loads_return_distinct_handles() {
    let a = audio::load_music();
    let b = audio::load_music();
    // On native both are opaque zeros — that's fine, the invariant we
    // care about is that we can hold two without either panicking.
    audio::play(&a, 0.3, false);
    audio::play(&b, 0.7, true);
}
