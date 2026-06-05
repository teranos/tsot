# ADR 0001 — Web Worker for wasm FFI

**Date:** 2026-06-05
**Status:** Accepted, shipped (commit 066305c)

## Decision

The wasm engine runs in a Web Worker. The main thread talks to it via
`postMessage`. FFI calls are async (Promise-resolved when the worker
posts back `{kind: 'envelope', json}`). Mid-call observability —
`tsot_emit_iteration_event` from inside `pick_play_uct` — goes
worker → main as `{kind: 'uct_iter', line}` while the FFI is still
blocked in worker scope.

Files: `assets/tsot-worker.js`, `assets/wasm-worker-lib.js`,
`assets/play.html` (Worker spawn + async `ffiCall`),
`.cargo/config.toml` (`--js-library=assets/wasm-worker-lib.js`),
`src/sim/uct.rs` (extern declaration + emit call).

## Comparison

The three candidate mechanisms for "FFI that can yield to JS and/or
post live progress mid-call":

|                                       | Web Worker | JSPI | Asyncify |
|---------------------------------------|------------|------|----------|
| Universal browser support             | yes        | no (~24% global, June 2026: Chrome/Edge/Opera 137+ only) | yes |
| Zero wasm-size penalty                | yes        | yes  | no (~30-50% bloat) |
| Mobile (iOS WebView + Android WebView)| yes        | **no** | yes |
| Live mid-call observability           | yes (postMessage from wasm) | partial (single-threaded; yield-only) | partial (single-threaded; yield-only) |
| Main thread free during long search   | yes        | no   | no  |

The two non-Worker mechanisms are single-threaded yield primitives:
the wasm pauses, JS does work, wasm resumes. They share two
disqualifying properties for TSOT:

1. **They don't free the main thread.** An 80-second UCT search still
   freezes the UI between yield points.
2. **They tie live observability to instrumented yield points.** Every
   place we want to surface progress has to become a yield.

A Worker sidesteps both: the wasm runs to completion on its own thread,
and any code inside that wasm can `postMessage` freely to the main
thread without yielding.

## Costs accepted

- Every FFI call becomes an async `postMessage` round-trip; `play.html`
  rewritten for `await`.
- One inflight FFI call at a time, tracked by a single `pendingFfi`
  slot. Engine is single-threaded, so no queue needed.
- Sub-millisecond per-call latency on the message bridge.
- `tsot-worker.js` is a small additional file shipped in `dist/`.

## What this also gave us, for free

- **No `__cxa_find_matching_catch_*` exception-ABI drama.** The
  original D4 path tried Asyncify and hit this against the precompiled
  Rust stdlib. The Worker path doesn't touch wasm rewriting, so the ABI
  question never arises.
- **Standard browser primitive.** No emscripten-specific instrumentation
  pass in the build pipeline.

## Why not JSPI specifically

JSPI is the modern, wasm-native replacement for Asyncify (Stage 4,
shipped in Chrome/Edge/Opera 137+ as of June 2026). It is *not* legacy
work — it's the recommended Chromium path for the synchronous-yield
use case. We don't use it because:

1. **No mobile.** iOS Safari, Android Chrome WebView, and all other
   mobile engines don't ship it. Capacitor's mobile reach (WASM_PLAN
   F1–F5) needs both WebViews.
2. **Single-threaded.** Same UI-blocking problem as Asyncify.

If TSOT were desktop-Chromium only and didn't have a long-running
search, JSPI would be the right answer.

## Why not Asyncify specifically

Asyncify (Binaryen pass; primary maintainer Alon Zakai @ Google;
https://github.com/WebAssembly/binaryen) is the portable, universal-
compat option for the same problem. It works. It's not deprecated.
We don't use it because:

1. ~30–50% wasm size penalty — bad for the mobile target (WASM_PLAN G2).
2. Exception-ABI mismatch against precompiled Rust stdlib (the original
   D4 blocker).
3. Single-threaded — same UI-blocking problem as JSPI.

## Notes

- The choice stops looking like a choice once STATE_MACHINE.md S6
  shipped (StepEngine made wasm pause/resume across FFI calls trivial,
  removing the original motivation for Asyncify).
- The choice becomes obvious once "live observability of mid-FFI UCT
  progress" enters the requirements — Worker is the only candidate
  that surfaces it without per-emit yield instrumentation.

## References

- WASM_PLAN.md G4 (this ADR satisfies it).
- STATE_MACHINE.md S6 (the StepEngine refactor that made FFI pause/
  resume orthogonal to the yield mechanism).
- caniuse JSPI: https://caniuse.com/wf-wasm-jspi
- Binaryen / Asyncify: https://github.com/WebAssembly/binaryen
- Emscripten Asyncify docs: https://emscripten.org/docs/porting/asyncify.html
- V8 JSPI: https://v8.dev/blog/jspi-newapi
