// Orchestrator. Wires every window-level bridge the wasm side expects,
// streams the wasm bytes with progress, then hands the bytes to
// wasm-bindgen's init() so the Bevy app boots. Once init resolves the
// loading indicator hides; Bevy's render loop owns the canvas from
// that point on.

import "./bridges";
import { showErr, installGlobalHandlers } from "./overlay";
import { installErrorBridges } from "./error-bridge";
import { installIdentityBridges } from "./identity-bridge";
import { installScreenshotBridge } from "./screenshot";
import { streamWasmBytes, hideLoadingIndicator } from "./loading";

// `WASM_URL_PLACEHOLDER` is substituted by the rave Makefile after the
// content-hashed wasm filename is known. Stays as the literal token
// at source-time so TypeScript / Bun bundling don't choke on it.
const WASM_URL = "WASM_URL_PLACEHOLDER";
// Same shape for the wasm-bindgen-generated JS glue.
const WASM_BINDGEN_JS = "./rave.js";

installGlobalHandlers();
installErrorBridges();
installIdentityBridges();
installScreenshotBridge();

try {
  const wasmBytes = await streamWasmBytes(WASM_URL);
  const { default: init } = await import(/* @vite-ignore */ WASM_BINDGEN_JS);
  await init({ module_or_path: wasmBytes });
  hideLoadingIndicator();
} catch (e) {
  showErr(`[init failed] ${e}`);
  if (e instanceof Error && e.stack) showErr(e.stack);
}
