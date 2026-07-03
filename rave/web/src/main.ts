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

// WebGPU preflight in JS BEFORE wasm loads. Same-origin JS catches see
// full error detail — bypasses the WebKit muting that hides wasm-
// originated throws. If Bevy's later WebGPU request is what mutes as
// `Script error.`, this preflight surfaces the same failure with real
// message, name, and cause. Deliberate destroy() at the end so we
// don't hold a device handle Bevy then can't get.
async function preflightWebGPU(): Promise<void> {
  if (!("gpu" in navigator)) {
    showErr("[preflight] navigator.gpu missing — no WebGPU on this browser");
    return;
  }
  try {
    showErr("[preflight] requesting GPU adapter...");
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) {
      showErr("[preflight] requestAdapter returned null — no compatible adapter");
      return;
    }
    showErr(
      `[preflight] adapter received: features=${adapter.features.size} limits.maxTextureDim2D=${adapter.limits.maxTextureDimension2D}`,
    );
    showErr("[preflight] requesting GPU device...");
    const device = await adapter.requestDevice();
    showErr(
      `[preflight] device received OK: features=${device.features.size} label=${JSON.stringify(device.label)}`,
    );
    // Wire uncapturederror on the preflight device so any subsequent
    // validation error before we destroy it also lands in the drawer.
    device.addEventListener("uncapturederror", (ev) => {
      const err = (ev as GPUUncapturedErrorEvent).error;
      showErr(`[preflight.uncaptured] ${err.constructor.name}: ${err.message}`);
    });
    device.destroy();
    showErr("[preflight] device destroyed cleanly");
  } catch (e) {
    showErr("[preflight] WebGPU preflight threw:");
    dumpError("[preflight.error]", e);
  }
}
await preflightWebGPU();

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
} catch (e) {
  showErr("[init] CAUGHT ERROR in try block — dumping now");
  dumpError("[init failed]", e);
}
