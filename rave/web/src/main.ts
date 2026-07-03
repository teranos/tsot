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

// Build identity is the FIRST thing to hit the drawer, before any
// init step, so the top of every screenshot self-identifies which
// bundle is running. No inference from wasm content-hashes required.
showErr(`[build] wasm=${WASM_URL} glue=${WASM_BINDGEN_JS}`);

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

  // Synthetic legibility anchor. Two deliberate throws so the next
  // mobile screenshot resolves whether iOS WebKit strips ALL
  // same-origin script errors from window.error (making all of them
  // `Script error.` with null details) or only some specific path.
  // Caught throw goes through the same dumper — proves in-JS
  // catches see full details. Uncaught setTimeout throw goes through
  // window.error — tells us what the browser boundary strips.
  setTimeout(() => {
    try {
      throw new Error("SYNTHETIC-CAUGHT — legibility anchor: caught in try");
    } catch (caught) {
      dumpError("[synthetic-caught]", caught);
    }
    setTimeout(() => {
      throw new Error("SYNTHETIC-UNCAUGHT — legibility anchor: hits window.error");
    }, 100);
  }, 3000);
} catch (e) {
  showErr("[init] CAUGHT ERROR in try block — dumping now");
  dumpError("[init failed]", e);
}
