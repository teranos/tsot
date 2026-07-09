# roam — observability

Handoff: seer + the env.* seam replace this doc's "narrate every internal decision" premise — Events/Health/Health-contract move to seer's docs, "errors are first-class" moves to game/CLAUDE.md as design axiom.

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
- [x] `TraceEvent` enum in `src/trace.rs` (variants: `Init`, `Note`, `Overflow`, `Error`; hand-written `to_json` per variant)
- [x] Thread-local buffer (`RefCell<VecDeque<(u64, TraceEvent)>>`) with overflow tracking
- [x] `roam_drain_trace` FFI returns JSON of buffered events; JS drains every frame
- [ ] Card-pickup arbitration, Lua handler invocations, state mutations emit events  *(partial: state mutations like `set_position` and `flower_picked_*` emit; card-pickup arbitration is v0.4 scope; Lua isn't in roam)*
- [x] Panel interleaves Rust + JS events by wall-clock timestamp
- [x] Rust panics caught via `panic_hook`, surfaced as `TraceEvent::Error` with file:line

### Phase 3 — Replay (v0.5+)
- [ ] Trace + initial seed → reproducible replay
- [ ] "Copy trace to clipboard" button → JSON dump for issue / fixture
- [ ] Time-travel scrubber in panel (drag back to any past event)

### Phase 4 — Health (durable conditions)

Events are ephemeral — a `Note` describes what just happened, scrolls
past, gone. Conditions are durable — what's wrong *right now*. The
log answers "what happened." A Health entry answers "what is the
current state."

Each Health entry declares:

- A **trigger** — the signal that creates or refreshes it.
- A **resolution rule** — the signal that closes it. Without one,
  the entry isn't a diagnosis; it's wallpaper.
- A **key** — what to upsert against (topic, peer, singleton).

Rendered as a "Current Health" panel — one row per active condition,
age clock, occurrence count tucked at the right. Resolved entries
linger briefly in a "recent" section, then drop. Augments (or
replaces) the placeholder `GOSSIPSUB MESH — TOPIC PEERS` panel
that currently sits empty most of the time.

First likely candidates — surfaced by the 0.3.6 publish-flood
incident:

- [ ] `PublishFailing { topic }` — onset: `NetEvent::Error(PublishFailed)` on the topic; resolution: next `Message` on that topic (any direction proves the mesh works).
- [ ] `MeshEmptyForTopic { topic }` — onset: derived from gossipsub `Subscribed`/`Unsubscribed` events leaving zero subscribers; resolution: any `Subscribed`.

Each new condition adds a variant + a trigger + a resolution rule
in the same edit. Drift between any of the three is the failure
mode that makes Health worse than logs, not better.

## Event categories (visual)

- `connect` (green) — peer/connection established
- `disconnect` (red) — peer/connection lost
- `sub` (cyan) — gossipsub subscription change
- `msg` (default) — pubsub message received
- `error` (bright red) — anything caught; includes stack
- `info` (grey) — lifecycle, init, multiaddr changes
