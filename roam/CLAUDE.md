roam — top-down P2P Rogue-like Deckbuilder MMO.

You roam an open world, gather TSOT cards lying on the ground, build
a deck from your collection, play TSOT matches against other players
you encounter automatically.

Architecture: Rust → wasm32-unknown-emscripten (mirrors TSOT). JS owns
network + render. js-libp2p over WebRTC for cross-browser, BroadcastChannel
for same-browser fallback. mlua scripting added in v0.4. No centralised
server. Decentralized substrate (libp2p), eventually-consistent state
(Lamport timestamps for card-pickup conflicts).

See @README.md for the staged roadmap and @OBSERVABILITY.md for the
trace bus plan.

**Errors are sacred** — first-class citizens, never collapsed, dropped,
or suppressed. They land in the event log panel with every other event,
in red, with full context (message + stack + the trace that led up to
them). No silent `.catch(() => {})`, no `console.warn`-and-move-on.
If you can't handle an error, surface it; never hide it.

**Observability first** — every meaningful action in the system emits
a structured event into the trace bus. The UI renders the bus. No
"why is this happening" mysteries: read the log, don't guess.

**Don't take the path of least resistance.**

**Hard rules (apply from day one):**

- No errors silencing or swallowing ever.
- No `console.warn` / `console.error` without ALSO pushing to the
  user-visible event log
- Every wasm FFI call that can fail returns an error envelope, never
  panics into an emscripten abort
- Every JS-side async error → captured by `window.onerror` or
  `unhandledrejection` handler → logged
- Errors keep their stack trace through every layer
- The user can read the log to see what happened; they should never
  need to open devtools to find a hidden cause

When the wasm side grows beyond `World::step`, mirror TSOT's pattern:
structured `TraceEvent` enum in `src/trace.rs`, thread-local buffer,
drained across the FFI per call, interleaved with JS events in the
panel by wall-clock timestamp.

Single-line git commits, no Claude attribution, no "Generated with"
footers. Commits only when the user asks.
