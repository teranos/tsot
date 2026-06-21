# roam — observability

Mirrors TSOT's principle: the system narrates every internal decision,
errors are first-class, no hidden state behind devtools. The event log
panel is the source of truth for "what just happened."

See @CLAUDE.md for the hard rules.

## Status (v0.3)

JS-only trace: the event log panel renders libp2p events (peer:connect,
peer:disconnect, connection:open, subscription-change, message), plus
all caught errors. No Rust-side trace yet — `World::step` is pure math
with nothing interesting to report.

## Plan

### Phase 1 — JS-side bus (v0.3)
- [x] Event log panel in the page (no devtools needed)
- [x] libp2p lifecycle events captured (peer:connect/disconnect, connection:open/close, peer:discovery, self:peer:update)
- [x] Gossipsub subscription-change captured per peer
- [x] `window.onerror` + `unhandledrejection` captured to panel
- [x] All silent `.catch(() => {})` removed; every error logs with cause
- [x] Each remote peer message logged (publish failures + receive parse errors)
- [x] HUD shows real diagnostic state (conns, mesh, multiaddrs)

### Phase 2 — Rust trace bus (lands with v0.4 when there's something to trace)
- [ ] `TraceEvent` enum in `src/trace.rs` (serde-serializable)
- [ ] Thread-local `RefCell<Vec<TraceEvent>>` buffer
- [ ] `roam_drain_trace` FFI returns JSON of buffered events; JS drains every frame
- [ ] Card-pickup arbitration, Lua handler invocations, state mutations emit events
- [ ] Panel interleaves Rust + JS events by wall-clock timestamp
- [ ] Rust panics caught via `panic_hook`, surfaced as `TraceEvent::Error` with file:line

### Phase 3 — Replay (v0.5+)
- [ ] Trace + initial seed → reproducible replay
- [ ] "Copy trace to clipboard" button → JSON dump for issue / fixture
- [ ] Time-travel scrubber in panel (drag back to any past event)

## Event categories (visual)

- `connect` (green) — peer/connection established
- `disconnect` (red) — peer/connection lost
- `sub` (cyan) — gossipsub subscription change
- `msg` (default) — pubsub message received
- `error` (bright red) — anything caught; includes stack
- `info` (grey) — lifecycle, init, multiaddr changes
