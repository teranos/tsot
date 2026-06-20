roam — top-down P2P Rogue-like Deckbuilder MMO.

**The only command that matters for the developer's workflow is:**

```
nix develop -c make wasm-serve
```

Everything else — feature flags, alt build targets, URL params, verification
matrices, `make wasm-rust` style alternates — is noise. Don't propose it, don't
add it, don't make the developer wade through it. If a change requires the
developer to type something other than that one command, the change is wrong.

You roam an open world, gather TSOT cards lying on the ground, build
a deck from your collection, play TSOT matches against other players
you encounter automatically.

Architecture: Rust → wasm32-unknown-emscripten (mirrors TSOT). JS owns
render and a thin `WorkerBridge` to the network web worker. The Swarm
(rust-libp2p, gossipsub + identify + ping over WebSocket-WebSys) lives
in `assets/src/net-worker.js`. No centralised server. Decentralized
substrate (libp2p), eventually-consistent state (Lamport timestamps
for card-pickup conflicts). Roadmap lives in `README.md`.

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

**JS is used in spite, not by choice.** It exists only because the
browser refuses to let wasm call `gl.drawArrays`, `canvas.getContext`,
`localStorage.setItem`, `libp2p.dial`, or `addEventListener` directly.
Every line of JS in this project must be one of:

1. A direct call to a browser API wasm cannot reach
2. Init / teardown of (1)
3. Byte-shoveling between (1) and wasm

Anything else — game state, render decisions, protocol parsing, color
tables, geometry, persistence schemas, inventory display — is a Rust
responsibility. If JS contains it, it's a bug, regardless of whether
the code works.

Adding logic to JS because "it's faster to write there" violates
*don't take the path of least resistance*. Write it in Rust. If the
FFI is in the way, fix the FFI.

**No stringly-typed FFI.** Wasm/JS boundary uses shared linear memory
with typed structs (bincode or hand-laid byte layouts read with
`DataView`). No JSON parallel-strings, no char-packed enums, no
`parseInt(s[i])`. If the Rust side has an enum, the JS side reads it
as an integer with a lookup table — never as a character.

**No positional tuples across the boundary.** `(u8, u8, u8, u8, u8)`
where each index means something different is a struct in disguise.
Make it a struct. Adding a field should be one edit, not five.

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
