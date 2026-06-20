# ADR 0002 — Elm for the in-browser dev tool

**Date:** 2026-06-07
**Status:** Accepted, shipping in stages (see `ELM_PLAN.md`)

## Context

TSOT is greenfield and wasm-first (ADR-0001). The in-browser dev tool
grew to ~2150 lines of inline JS in `assets/play.html` alongside the
engine. Its top-level state is a finite phase machine
(`deckbuilding | playing | spectating`) and the surface to the wasm
worker is small and ours.

The trade space is about minimizing untyped surface, not about
ergonomics on synchronous code we cannot rewrite.

## Decision

The dev tool is written in Elm. A thin JS bridge (`assets/js-bridge.js`)
crosses what Elm cannot touch directly — Web Worker, IndexedDB, file
pickers, `Atomics.store` on the shared heap — via envelope ports
(`{cmd, payload}`).

Files: `assets/src/Main.elm`, `assets/src/elm.json`,
`assets/js-bridge.js`, `assets/play.html` (markup + worker bootstrap),
`Makefile` `assets` target.

## Comparison

|                                       | Elm | TypeScript | Rust → Yew/Leptos | PureScript | ReScript |
|---------------------------------------|-----|------------|-------------------|------------|----------|
| Compile-time guarantees on transitions | yes | no | yes | yes | partial |
| No runtime exceptions by construction  | yes | no | yes | yes | no |
| Bundle output                          | small JS | JS | second wasm | JS | JS |
| Iteration loop                         | `elm make` | `tsc` | `cargo build` + wasm-pack | `spago build` | `rescript build` |
| The Elm Architecture as the only program shape | yes | no | no | no | no |

The decision driver is the last row. Elm has no JS escape hatches and
no runtime exception path; `undefined is not a function` is structurally
impossible. The Elm Architecture is the only shape the language
supports, and that shape is the dev tool's shape.

## Costs accepted

- Hand-written `Json.Decode` for every FFI envelope; schema drift
  surfaces as a runtime decoder failure into the LOG, not at compile
  time.
- The JS bridge layer cannot be eliminated; Elm 0.19 doesn't speak
  Worker / IDB / file pickers natively.
- `elm make` joins `cargo` / `lua5.4` / `emcc` in the toolchain;
  `flake.nix` provides it.
- Hand-written port plumbing on the JS side.

## Why not TypeScript

Types are advisory; the runtime is JavaScript. `any` and `as` silence
the checker. Phase transitions are enforced by convention, not by the
language.

## Why not Rust → Yew / Leptos / Dioxus

Second wasm bundle alongside the engine's. `cargo` + wasm-pack
iteration loop is slow. The Rust UI ecosystem still needs a JS shim
for browser APIs, so we'd pay the bridge cost without gaining
discipline on UI state transitions Elm gives for free.

## Why not PureScript

Higher-kinded types, monad transformers, type classes — abstractions
the dev tool will not use. Cognitive ceiling outpaces what we'd
exercise; smaller ecosystem than Elm.

## Why not ReScript

Less prescriptive than Elm. Mutable bindings, `%raw`, and
`Js.Obj.empty()` let JS-shaped code bypass the type system. Where
ReScript encourages discipline, Elm enforces it; for a phase-machine
UI, enforcement is the right tool.

## Notes

- Envelope ports (one outbound `cmd` channel per side-effect kind,
  dispatch by string) are a first-principles consequence of
  minimizing untyped surface. Per-feature ports grow linearly with
  features; envelope ports are constant.
- `ELM_PLAN.md` tracks the migration stages. This ADR records the
  framework choice. They cite each other.

## References

- `ELM_PLAN.md`
- ADR-0001 (Web Worker for wasm FFI)
- Elm Guide: https://guide.elm-lang.org
- The Elm Architecture: https://guide.elm-lang.org/architecture/
- ReScript: https://rescript-lang.org
- PureScript: https://www.purescript.org
- Yew: https://yew.rs, Leptos: https://leptos.dev
