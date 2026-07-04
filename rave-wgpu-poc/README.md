# rave-wgpu-poc

A spike: render a minimal slice of the rave world (forest floor +
Wang-hash trees + follow camera) with **wgpu 29 directly** — no Bevy,
no winit — to test whether abandoning Bevy-rave's renderer for wgpu is
justified.

- One wasm binary holds **both** WebGPU and WebGL2 (runtime-selected),
  which Bevy's compile-time shader gating could not do.
- Deliberately LDR (no HDR / bloom / MSAA) to isolate the render
  features implicated in Bevy-rave's mobile OOM.

See **[HANDOVER.md](./HANDOVER.md)** for what's verified, what isn't,
how to build/run, and how the result feeds the Bevy-vs-wgpu decision.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.121
cd rave-wgpu-poc && ./web/build.sh && python3 -m http.server -d dist 8080
```
