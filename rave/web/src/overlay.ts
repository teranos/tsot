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

// Catch any uncaught error or unhandled promise rejection in JS so
// the user sees it instead of having to open devtools.
export function installGlobalHandlers(): void {
  window.addEventListener("error", (ev: ErrorEvent) => {
    const where = ev.filename
      ? ` @ ${ev.filename}:${ev.lineno}:${ev.colno}`
      : "";
    showErr(`[window.error] ${ev.message}${where}`);
    if (ev.error?.stack) showErr(ev.error.stack);
  });
  window.addEventListener("unhandledrejection", (ev: PromiseRejectionEvent) => {
    showErr(`[unhandledrejection] ${ev.reason}`);
    if (ev.reason?.stack) showErr(ev.reason.stack);
  });
}
