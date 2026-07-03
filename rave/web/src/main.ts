// Orchestrator. Wires every window-level bridge the wasm side expects,
// streams the wasm bytes with progress, then hands the bytes to
// wasm-bindgen's init() so the Bevy app boots. Once init resolves the
// loading indicator hides; Bevy's render loop owns the canvas from
// that point on.

import { showErr, dumpError, installGlobalHandlers } from "./overlay";
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

interface RaveWasmExports {
  default: (opts: { module_or_path: Uint8Array }) => Promise<unknown>;
  rave_drawer_toggle: () => void;
}

// Step-by-step trace so the drawer shows the exact last-successful
// milestone before any failure — no need to rely on window.onerror
// which iOS WebKit CORB-sanitises for module errors. Each `showErr`
// writes to the visible overlay directly.
showErr(`[init] step 0 — user-agent: ${navigator.userAgent}`);
showErr(`[init] step 0 — navigator.gpu present: ${'gpu' in navigator}`);

installGlobalHandlers();
showErr("[init] step 1 — global error handlers installed");

installErrorBridges();
showErr("[init] step 2 — error bridges installed");

installIdentityBridges();
showErr("[init] step 3 — identity bridges installed");

installScreenshotBridge();
showErr("[init] step 4 — screenshot bridge installed");

try {
  showErr(`[init] step 5 — fetching wasm from ${WASM_URL}`);
  const wasmBytes = await streamWasmBytes(WASM_URL);
  showErr(`[init] step 6 — wasm bytes received (${wasmBytes.byteLength} bytes)`);

  showErr(`[init] step 7 — importing wasm-bindgen glue ${WASM_BINDGEN_JS}`);
  const wasm = (await import(
    /* @vite-ignore */ WASM_BINDGEN_JS
  )) as RaveWasmExports;
  showErr("[init] step 8 — wasm-bindgen glue imported");

  showErr("[init] step 9 — calling wasm.default() to instantiate");
  await wasm.default({ module_or_path: wasmBytes });
  showErr("[init] step 10 — wasm instantiated, Bevy owns the canvas");

  hideLoadingIndicator();
  showErr("[init] step 11 — loading indicator hidden");

  // Chat is now a Bevy plugin (bevy_chat::ChatOverlayPlugin from laye).
  // No DOM hook needed — canvas UI handles focus + typing + history.

  // Drawer touch toggle is now a Bevy-UI ≡ button inside the
  // minimap (see `rave::minimap::MinimapToggleButton`). No DOM
  // hook needed here.
} catch (e) {
  showErr("[init] CAUGHT ERROR in try block — dumping now");
  dumpError("[init failed]", e);
}
