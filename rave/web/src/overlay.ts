// Out-of-Bevy error surface. Visible even when wasm fails to
// instantiate or Bevy panics before its first Update tick. The
// in-canvas drawer only works if Bevy's render loop is alive;
// this is the catch-all that doesn't depend on it.
//
// State survives reload cycles via sessionStorage. If the tab is
// reloading itself (memory pressure, WebGPU context loss, whatever)
// the previous cycle's log gets restored at page load, so we see
// what happened before the reset. Every line carries `[+Nms]`
// since page load — that plus the `load#N` header makes the timeline
// unambiguous.

const el = document.getElementById("bevy-error") as HTMLDivElement | null;
const T0 = performance.now();
const LOG_STORAGE_KEY = "rave-drawer-log";
const LOAD_COUNTER_KEY = "rave-load-counter";

// Substituted at bundle time by `rave/web/hash-and-stage.sh` from
// `RAVE_BUILD_COMMIT` + `RAVE_BUILD_TIME` env vars the Makefile sets.
// Stays as the literal token at source-time so bundlers don't choke.
const RAVE_COMMIT = "RAVE_COMMIT_PLACEHOLDER";
const RAVE_BUILT_AT = "RAVE_BUILT_AT_PLACEHOLDER";

function nextLoadNumber(): number {
  try {
    const prev = parseInt(sessionStorage.getItem(LOAD_COUNTER_KEY) ?? "0", 10);
    const next = Number.isFinite(prev) ? prev + 1 : 1;
    sessionStorage.setItem(LOAD_COUNTER_KEY, String(next));
    return next;
  } catch {
    return -1;
  }
}

function navigationType(): string {
  try {
    const entries = performance.getEntriesByType(
      "navigation",
    ) as PerformanceNavigationTiming[];
    return entries[0]?.type ?? "unknown";
  } catch {
    return "unknown";
  }
}

const LOAD_NUMBER = nextLoadNumber();
const NAV_TYPE = navigationType();

if (el) {
  el.style.display = "block";
  // Restore any log from a previous load cycle so a reload loop leaves
  // its footprint. sessionStorage is scoped to the tab and survives
  // reloads; it clears when the tab closes.
  try {
    const previous = sessionStorage.getItem(LOG_STORAGE_KEY);
    if (previous) {
      el.textContent = previous + `--- reload boundary ---\n`;
    }
  } catch {
    /* storage disabled — proceed with empty */
  }
}

export function showErr(line: string): void {
  if (!el) return;
  const stamped = `[+${Math.round(performance.now() - T0)}ms] ${line}\n`;
  el.textContent = (el.textContent ?? "") + stamped;
  try {
    sessionStorage.setItem(LOG_STORAGE_KEY, el.textContent ?? "");
  } catch {
    /* quota exceeded or storage disabled — line still visible on-screen */
  }
  // Auto-scroll the document so the latest line is always in the
  // viewport. Without this, the drawer with max-height:none extends
  // past the visible viewport and CI screenshots always capture the
  // TOP of the drawer — never the [mem@Ns] lines that fire later.
  try {
    window.scrollTo({ top: document.body.scrollHeight, behavior: "instant" });
  } catch {
    /* older browsers — behavior:instant unsupported, ignore */
  }
}

// Absolute first drawer line — commit + build time, distinct enough
// that no screenshot can hide which bundle is running. Placed BEFORE
// the load header so it's the very top of every screenshot, even
// truncated ones. Substituted at build time by hash-and-stage.sh.
showErr(`=== rave ${RAVE_COMMIT} · built ${RAVE_BUILT_AT} ===`);

// Second line — load number + navigation type + wall clock. A rapidly
// climbing load# is proof of a reload loop.
showErr(
  `=== load#${LOAD_NUMBER} nav=${NAV_TYPE} @ ${new Date().toISOString()} ===`,
);

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
  // iOS WebKit sanitises `Script error.` and null `ev.error` even
  // for same-origin module scripts with ACAO present. Dumping every
  // own property of the event exposes anything the platform hasn't
  // decided to strip (vendor extensions, timestamps, event phase).
  const dumpErrorEvent = (phase: string) => (ev: ErrorEvent): void => {
    showErr(
      `[window.error ${phase}] msg=${JSON.stringify(ev.message)} file=${JSON.stringify(ev.filename)} line=${ev.lineno} col=${ev.colno}`,
    );
    showErr(
      `  ev meta: type=${ev.type} isTrusted=${ev.isTrusted} timeStamp=${ev.timeStamp} eventPhase=${ev.eventPhase}`,
    );
    for (const key of Object.getOwnPropertyNames(ev)) {
      try {
        const v = (ev as unknown as Record<string, unknown>)[key];
        if (v == null) continue;
        if (typeof v === "function") continue;
        if (typeof v === "object") continue;
        showErr(`  ev.${key} = ${JSON.stringify(v)}`);
      } catch (accessorThrew) {
        showErr(`  ev.${key} <accessor threw: ${accessorThrew}>`);
      }
    }
    if (ev.error) dumpError("[window.error.error]", ev.error);
    else showErr(`  <ev.error was null — browser refused to attribute; likely wasm-originated throw>`);
  };
  window.addEventListener("error", dumpErrorEvent("bubble"), false);
  window.addEventListener("error", dumpErrorEvent("capture"), true);
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
