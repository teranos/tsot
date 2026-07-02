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
// `cause`). For CORB-sanitized errors the message is gone but other
// properties sometimes survive. Better than `${e}` which collapses
// to the sanitized string.
//
// Errors are first-class primitives (`ERROR.md`); the developer sees
// them or the axiom is violated. This dumper walks every enumerable-
// or-not own property, recurses into `.cause` (Error chains), quotes
// arrays, and never gives up on partial data.
function dumpError(prefix: string, e: unknown, depth: number = 0): void {
  const indent = "  ".repeat(depth);
  if (e == null) {
    showErr(`${indent}${prefix} <null/undefined>`);
    return;
  }
  if (typeof e !== "object") {
    showErr(`${indent}${prefix} ${typeof e}: ${String(e)}`);
    return;
  }
  const obj = e as Record<string, unknown>;
  const ctor = (obj.constructor as { name?: string } | undefined)?.name ?? "Object";
  const strForm = tryToString(e);
  showErr(`${indent}${prefix} ${ctor}: ${strForm}`);
  for (const key of Object.getOwnPropertyNames(obj)) {
    const v = obj[key];
    if (v === undefined) continue;
    if (v === null) {
      showErr(`${indent}  .${key} = null`);
      continue;
    }
    if (typeof v === "string") {
      if (key === "stack") {
        showErr(`${indent}  .stack:`);
        for (const line of v.split("\n")) showErr(`${indent}    ${line}`);
      } else {
        showErr(`${indent}  .${key} = ${v}`);
      }
      continue;
    }
    if (typeof v === "number" || typeof v === "boolean") {
      showErr(`${indent}  .${key} = ${v}`);
      continue;
    }
    if (typeof v === "bigint" || typeof v === "symbol") {
      showErr(`${indent}  .${key} = ${String(v)} (${typeof v})`);
      continue;
    }
    if (Array.isArray(v)) {
      showErr(`${indent}  .${key} [${v.length}]:`);
      v.forEach((item, i) => dumpError(`[${i}]`, item, depth + 2));
      continue;
    }
    if (key === "cause" && depth < 4) {
      showErr(`${indent}  .cause:`);
      dumpError("caused by", v, depth + 2);
      continue;
    }
    if (depth < 3) {
      dumpError(`.${key}`, v, depth + 1);
      continue;
    }
    showErr(`${indent}  .${key} = <${typeof v}>`);
  }
}

// String-cast that survives errors WITH a toString that throws
// (rare but seen in the wild). Falls back to constructor name.
function tryToString(e: unknown): string {
  try {
    return String(e);
  } catch {
    return "<toString threw>";
  }
}

// Catch any uncaught error or unhandled promise rejection in JS so
// the user sees it instead of having to open devtools. Per ERROR.md,
// errors are first-class primitives; if they aren't in front of the
// user, the axiom is violated.
export function installGlobalHandlers(): void {
  window.addEventListener("error", (ev: ErrorEvent) => {
    const where = ev.filename
      ? ` @ ${ev.filename}:${ev.lineno}:${ev.colno}`
      : "";
    showErr(`[window.error] ${ev.message}${where}`);
    // ev.error carries the real Error object (with stack, cause,
    // custom fields). For same-origin scripts this is populated;
    // dumpError walks it thoroughly. Only sanitized when the script
    // was loaded with `crossorigin="anonymous"` AND the response
    // lacks `Access-Control-Allow-Origin`.
    if (ev.error) dumpError("[window.error.error]", ev.error);
    else showErr(`  <ev.error was null — script was CORB-sanitized>`);
  });
  window.addEventListener("unhandledrejection", (ev: PromiseRejectionEvent) => {
    dumpError("[unhandledrejection]", ev.reason);
  });
  // Also hook console.error so anything that logs through the console
  // (Bevy panics, wasm-bindgen errors, third-party libs) also lands
  // in the overlay. Preserves the original console.error so devtools
  // still sees it too.
  const originalConsoleError = console.error.bind(console);
  console.error = (...args: unknown[]) => {
    originalConsoleError(...args);
    try {
      const line = args
        .map((a) => (typeof a === "string" ? a : tryToString(a)))
        .join(" ");
      showErr(`[console.error] ${line}`);
      for (const a of args) {
        if (a && typeof a === "object") {
          dumpError("[console.error.arg]", a);
        }
      }
    } catch (dumpFailure) {
      showErr(`[console.error dump failed] ${String(dumpFailure)}`);
    }
  };
}

// Re-exported so the init-side catch in main.ts uses the same dumper
// the global handlers do — one path to maintain.
export { dumpError };
