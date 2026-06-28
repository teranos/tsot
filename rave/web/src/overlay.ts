// Out-of-Bevy error surface. Visible even when wasm fails to
// instantiate or Bevy panics before its first Update tick. The
// in-canvas drawer only works if Bevy's render loop is alive;
// this is the catch-all that doesn't depend on it.

const el = document.getElementById("bevy-error") as HTMLDivElement | null;

export function showErr(line: string): void {
  if (!el) return;
  el.style.display = "block";
  el.textContent = (el.textContent ?? "") + line + "\n";
}

// Dumps every own property of an error-like object — including
// non-enumerable ones browsers attach (`stack`, `code`, `name`,
// `cause`). For "Script error." sanitization the message is gone
// but other properties (constructor name, WebAssembly-specific
// fields) sometimes survive. Better than `${e}` which collapses to
// the sanitized string.
function dumpError(prefix: string, e: unknown): void {
  if (e == null) {
    showErr(`${prefix} <null/undefined>`);
    return;
  }
  if (typeof e !== "object") {
    showErr(`${prefix} ${typeof e}: ${String(e)}`);
    return;
  }
  const obj = e as Record<string, unknown>;
  const ctor = (obj.constructor as { name?: string } | undefined)?.name ?? "Object";
  showErr(`${prefix} ${ctor}: ${String(e)}`);
  for (const key of Object.getOwnPropertyNames(obj)) {
    const v = obj[key];
    if (v == null) continue;
    if (typeof v === "string" || typeof v === "number" || typeof v === "boolean") {
      showErr(`  .${key} = ${v}`);
    } else if (key === "stack" && typeof v === "string") {
      showErr(`  .stack:`);
      showErr(v);
    }
  }
}

// Catch any uncaught error or unhandled promise rejection in JS so
// the user sees it instead of having to open devtools.
export function installGlobalHandlers(): void {
  window.addEventListener("error", (ev: ErrorEvent) => {
    const where = ev.filename
      ? ` @ ${ev.filename}:${ev.lineno}:${ev.colno}`
      : "";
    showErr(`[window.error] ${ev.message}${where}`);
    // "Script error." sanitization: ev.error is usually null when the
    // browser strips the message. dumpError handles both cases.
    if (ev.error) dumpError("[window.error.error]", ev.error);
  });
  window.addEventListener("unhandledrejection", (ev: PromiseRejectionEvent) => {
    dumpError("[unhandledrejection]", ev.reason);
  });
}

// Re-exported so the init-side catch in main.ts uses the same dumper
// the global handlers do — one path to maintain.
export { dumpError };
