// Bridges the wasm-side typed Error pipeline to the JS overlay.
// crate::error::emit_region (Rust) serialises a sacred_error::Error
// to JSON and calls window.__raveErrorTyped(json). This module
// decodes that JSON and surfaces the structured fields the axiom in
// ERROR.md cares about: severity, surface, region, title, why.

import { showErr } from "./overlay";

export interface TypedError {
  id: string;
  severity: "info" | "warn" | "error" | "panic";
  context: {
    surface: string;
    region?: string;
  };
  title: string;
  why: string;
}

export function installErrorBridges(): void {
  // Stringly-typed line surface. Reserved for hot paths the typed
  // pipeline can't reach (pre-Bevy panic hook, tracing layer).
  window.__raveError = showErr;

  // Typed surface. Wasm calls this with a JSON-serialised
  // sacred_error::Error; we decode and format with context.
  window.__raveErrorTyped = (json: string): void => {
    try {
      const e = JSON.parse(json) as TypedError;
      const region = e.context?.region ? "/" + e.context.region : "";
      const surface = e.context?.surface ?? "?";
      showErr(
        `[${e.severity}@${surface}${region}] ${e.title || ""} — ${e.why || ""}`,
      );
    } catch (parseErr) {
      showErr(`[__raveErrorTyped parse failed] ${parseErr}: ${json}`);
    }
  };
}
