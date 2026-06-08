// Main-thread JS bridge for the Elm dev-tool port. Owns whatever Web
// Platform surface the Elm app can't touch directly via ports — the
// wasm Worker handle, IndexedDB, SharedArrayBuffer atomic writes, file
// download, prompt/confirm, setInterval.
//
// H7-Elm Stage 2: `buildInfoIn` port carries `window.__TSOT_BUILD__`.
// H7-Elm Stage 3: `logTextIn` + `logErrorIn` ports carry every LOG
// event (text lines + structured error blocks).
// H7-Elm Stage 4: decision-report panel + first outbound port.
//   - `decisionFetchOut` (Elm → JS): Elm asks for the decision log
//     records. JS opens IDB, reads `decision_log`, sends back via
//     `decisionLogIn`.
//   - `decisionReportClickedIn` (JS → Elm): play.html button click
//     forwards the click; Elm decides toggle vs fetch.
//   - `window.tsotDecisionReport / Export / Clear`: shims that
//     play.html's three button `onclick` handlers call.
//
// IDB schema during the transition: this file and play.html both
// open `tsot` v2 with `saves` + `decision_log` stores. The schema is
// duplicated until a later Stage consolidates ownership; both sides
// agree on the upgrade so no schema drift can occur.

const TSOT_DB_NAME = 'tsot';
const TSOT_DB_VERSION = 2;
const TSOT_SAVES_STORE = 'saves';
const TSOT_DECISION_STORE = 'decision_log';

function tsotOpenDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(TSOT_DB_NAME, TSOT_DB_VERSION);
    req.onerror = () => reject(req.error);
    req.onsuccess = () => resolve(req.result);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(TSOT_SAVES_STORE)) {
        db.createObjectStore(TSOT_SAVES_STORE, { keyPath: 'id', autoIncrement: true });
      }
      if (!db.objectStoreNames.contains(TSOT_DECISION_STORE)) {
        db.createObjectStore(TSOT_DECISION_STORE, { keyPath: 'id', autoIncrement: true });
      }
    };
  });
}

async function tsotDbGetAllDecisions() {
  const db = await tsotOpenDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(TSOT_DECISION_STORE, 'readonly');
    const req = tx.objectStore(TSOT_DECISION_STORE).getAll();
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tsotDbClearDecisions() {
  const db = await tsotOpenDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(TSOT_DECISION_STORE, 'readwrite');
    const req = tx.objectStore(TSOT_DECISION_STORE).clear();
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
  });
}

// H7-Elm Stage 5 — saves-store read helpers. The write side
// (`dbPut`) still lives in play.html (called by `onSaveClick`); the
// read + delete sides come here so the Elm saved-list panel and its
// per-row Load/Delete/Download buttons can drive them via the
// `savedListFetchOut` + `savedItemActionOut` ports.
async function tsotDbGetAllSaves() {
  const db = await tsotOpenDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(TSOT_SAVES_STORE, 'readonly');
    const req = tx.objectStore(TSOT_SAVES_STORE).getAll();
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tsotDbGetSaveById(id) {
  const db = await tsotOpenDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(TSOT_SAVES_STORE, 'readonly');
    const req = tx.objectStore(TSOT_SAVES_STORE).get(id);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tsotDbDeleteSave(id) {
  const db = await tsotOpenDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(TSOT_SAVES_STORE, 'readwrite');
    const req = tx.objectStore(TSOT_SAVES_STORE).delete(id);
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
  });
}

// Strip the (potentially large) `json` field and sort newest-first.
// Elm only needs metadata for the list view; the json content is read
// on-demand inside Load / Download handlers below.
function tsotSavesToMetadataList(records) {
  return (records || [])
    .map((r) => ({ id: r.id, name: r.name, savedAt: r.savedAt }))
    .sort((a, b) => (b.savedAt || '').localeCompare(a.savedAt || ''));
}

function tsotSetSaveStatus(msg) {
  const el = document.getElementById('save-status');
  if (el) el.textContent = msg;
}

// ============================================================
// LOG + inline-error infrastructure (moved from play.html).
// These are top-level function declarations on global scope so
// existing call sites in play.html (worker message handler,
// fetchState's parseEnvelope chain, the bootstrap, withInlineError)
// keep working without rewrite. Errors are sacred — every path
// converges on the same `buildErrorBlock` DOM shape so an FFI
// failure looks identical whether it lands in the LOG (via the
// logErrorIn port) or inline next to the failing button (via
// renderErrorAt).
// ============================================================

// Mid-flight live UCT iteration event — arrives from worker's
// tsot_emit_iteration_event while the FFI call is still blocked.
// Rendered with a distinct `[live UctIter]` prefix so it's visually
// distinguishable from the post-envelope `[UctIter]` lines emitted
// by appendTrace once the call returns.
function appendLiveUctIter(line) {
  let formatted;
  try {
    const ev = JSON.parse(line);
    formatted = `[live UctIter] iter=${ev.iter}/${ev.total}  ${(ev.duration_us / 1000).toFixed(1)}ms  turns=${ev.rollout_turns} plays=${ev.rollout_plays} atks=${ev.rollout_attacks} deaths=${ev.rollout_deaths} fires=${ev.rollout_handler_fires}`;
  } catch (_) {
    formatted = `[live UctIter] ${line}`;
  }
  window.tsotLogPushText(formatted);
}

// Build a styled `.log-error` block from a `TraceEvent::Error`
// envelope. Same DOM regardless of where it will be inserted —
// callers decide the anchor: appendErrorEvent puts it in the LOG;
// renderErrorAt puts it right next to the button that failed.
// Errors are first-class observability events — same shape, full
// context (source, FFI call, message, location, breadcrumb),
// never collapsed, never truncated.
function buildErrorBlock(ev) {
  const block = document.createElement('div');
  block.className = 'log-error';
  const source = ev.source || 'error';
  const header = document.createElement('div');
  header.className = 'log-error-header';
  header.textContent = `[${source.toUpperCase()}] ${ev.message || '(no message)'}`;
  block.appendChild(header);
  const sub = document.createElement('div');
  sub.className = 'log-error-meta';
  const parts = [];
  if (ev.location) parts.push(`at ${ev.location}`);
  if (ev.ffi_call) parts.push(`inside FFI ${ev.ffi_call}`);
  if (typeof ev.at_us === 'number') parts.push(`t=${(ev.at_us / 1000).toFixed(1)}ms`);
  sub.textContent = parts.join('  ·  ');
  block.appendChild(sub);
  const trail = ev.recent_trace || [];
  if (trail.length) {
    const breadcrumb = document.createElement('div');
    breadcrumb.className = 'log-error-trail';
    breadcrumb.textContent = `--- last ${trail.length} trace events before failure ---`;
    block.appendChild(breadcrumb);
    for (const inner of trail) {
      const line2 = document.createElement('div');
      line2.className = 'log-error-trail-line';
      try { line2.textContent = fmtTraceEvent(inner); }
      catch (_) { line2.textContent = JSON.stringify(inner); }
      block.appendChild(line2);
    }
  }
  if (ev.js_stack) {
    const stackLabel = document.createElement('div');
    stackLabel.className = 'log-error-trail';
    stackLabel.textContent = '--- JS exception stack ---';
    block.appendChild(stackLabel);
    const stackPre = document.createElement('div');
    stackPre.className = 'log-error-trail-line';
    stackPre.style.whiteSpace = 'pre-wrap';
    stackPre.textContent = ev.js_stack;
    block.appendChild(stackPre);
  }
  if (ev.raw_stderr) {
    const stderrLabel = document.createElement('div');
    stderrLabel.className = 'log-error-trail';
    stderrLabel.textContent = '--- raw stderr from wasm ---';
    block.appendChild(stderrLabel);
    const stderrPre = document.createElement('div');
    stderrPre.className = 'log-error-trail-line';
    stderrPre.style.whiteSpace = 'pre-wrap';
    stderrPre.textContent = ev.raw_stderr;
    block.appendChild(stderrPre);
  }
  if (source === 'rust-panic' || source === 'wasm-trap') {
    const footer = document.createElement('div');
    footer.className = 'log-error-meta';
    footer.textContent = 'wasm module aborted after this point — reload the page to continue';
    block.appendChild(footer);
  }
  return block;
}

// Append the error block to the LOG (the audit trail). Pre-formats
// the recent_trace breadcrumb via fmtTraceEvent here (the formatter
// is in this same file), then pushes a structured envelope through
// the logErrorIn port. Elm rebuilds the styled .log-error block from
// the same fields — the `renderErrorAt` inline-error-next-to-button
// path uses `buildErrorBlock` directly so the DOM is the same shape.
function appendErrorEvent(ev) {
  const formatted = {
    source: ev.source || 'error',
    message: ev.message || '(no message)',
    location: ev.location || null,
    ffi_call: ev.ffi_call || null,
    at_us: typeof ev.at_us === 'number' ? ev.at_us : null,
    breadcrumb: (ev.recent_trace || []).map((inner) => {
      try { return fmtTraceEvent(inner); }
      catch (_) { return JSON.stringify(inner); }
    }),
    js_stack: ev.js_stack || null,
    raw_stderr: ev.raw_stderr || null,
  };
  window.tsotLogPushError(formatted);
}

// Display the error block immediately after `anchor` (the button or
// row that triggered the action). Replaces any previous inline
// error attached to the same anchor so the surface stays current.
function renderErrorAt(anchor, ev) {
  if (!anchor || !anchor.parentNode) return;
  if (anchor._inlineError && anchor._inlineError.parentNode) {
    anchor._inlineError.parentNode.removeChild(anchor._inlineError);
  }
  const block = buildErrorBlock(ev);
  block.classList.add('log-error-inline');
  anchor._inlineError = block;
  anchor.parentNode.insertBefore(block, anchor.nextSibling);
}

function clearInlineErrorAt(anchor) {
  if (!anchor) return;
  if (anchor._inlineError && anchor._inlineError.parentNode) {
    anchor._inlineError.parentNode.removeChild(anchor._inlineError);
  }
  anchor._inlineError = null;
}

// Build a `TraceEvent::Error`-shaped envelope from a JS-side
// exception. Used by every onclick that wraps an FFI call so we
// can flow JS-caught errors through the same renderer as Rust ones.
function jsErrorEnvelope(label, e) {
  return {
    kind: 'Error',
    at_us: 0,
    source: 'js',
    ffi_call: label,
    message: (e && e.message) ? e.message : String(e),
    location: null,
    recent_trace: [],
  };
}

// Append a batch of log lines through the logTextIn port. Used by
// fetchState's parseEnvelope to flush engine.log into the LOG panel.
function appendLog(lines) {
  if (!lines || lines.length === 0) return;
  for (const line of lines) {
    window.tsotLogPushText(line);
  }
}

// Format one TraceEvent as a single rendered line. Δms is `at_us /
// 1000` from the bus's session origin (set in Rust by
// `trace::enable(true)` on the first FFI call). The category prefix
// lets the reader scan the stream; the payload after is formatted
// per-variant.
function fmtTraceEvent(ev) {
  const t = (ev.at_us / 1000).toFixed(1).padStart(8);
  switch (ev.kind) {
    case 'Step':
      return `[${t}ms Step]      ${ev.from}  ->  ${ev.to}  (${(ev.duration_us / 1000).toFixed(2)}ms)  -> ${ev.result}`;
    case 'Cursor':
      return `[${t}ms Cursor]    ${ev.from}  ->  ${ev.to}`;
    case 'Phase':
      return `[${t}ms Phase]     turn=${ev.turn} ${ev.from}  ->  ${ev.to}`;
    case 'Mutation':
      return `[${t}ms Mutation]  ${JSON.stringify(ev.entry)}`;
    case 'Count':
      return `[${t}ms Count]     ${ev.key}[${ev.player}] ${ev.before}  ->  ${ev.after}`;
    case 'Oracle':
      return `[${t}ms Oracle]    ${ev.call}(asker=${ev.asker})  ->  ${ev.answer}  (${(ev.duration_us / 1000).toFixed(2)}ms)`;
    case 'Play':
      return `[${t}ms Play]      ${ev.iid}  ->  ${ev.outcome}  (${(ev.duration_us / 1000).toFixed(2)}ms)`;
    case 'Winner':
      return `[${t}ms Winner]    ${ev.who} wins  (cause: ${ev.cause})`;
    case 'Ffi':
      return `[${t}ms Ffi]       ${ev.span}  (${(ev.duration_us / 1000).toFixed(2)}ms)`;
    case 'AiPick': {
      const top = (ev.candidates || []).slice(0, 6)
        .map(c => `${c.iid}=${c.score}`)
        .join(', ');
      const more = (ev.candidates || []).length > 6 ? ` +${ev.candidates.length - 6} more` : '';
      return `[${t}ms AiPick]    ${ev.ai}  candidates=[${top}${more}]  -> ${ev.chosen || '(none)'}  (${(ev.duration_us / 1000).toFixed(2)}ms)`;
    }
    case 'AttackerSelection': {
      const elig = (ev.eligible || []).join(',');
      const chosen = (ev.chosen || []).join(',');
      return `[${t}ms Attackers] ${ev.player}: eligible=[${elig}]  chosen=[${chosen}]  (${(ev.duration_us / 1000).toFixed(2)}ms)`;
    }
    case 'BlockerSelection': {
      const atks = (ev.attackers || []).join(',');
      const pairs = (ev.assignments || []).map(p => `${p[0]}->${p[1]}`).join(', ');
      return `[${t}ms Blockers]  ${ev.defender}: attackers=[${atks}]  blocks=[${pairs}]  (${(ev.duration_us / 1000).toFixed(2)}ms)`;
    }
    case 'Handler': {
      const partner = ev.partner ? ` partner=${ev.partner}` : '';
      const err = ev.error ? `  ERR: ${ev.error}` : '';
      return `[${t}ms Handler]   ${ev.event}  source=${ev.source}${partner}  (${(ev.duration_us / 1000).toFixed(2)}ms)${err}`;
    }
    case 'UctIteration': {
      const path = (ev.path || []).join(' -> ');
      return `[${t}ms UctIter]   iter=${ev.iter}/${ev.total}  ${(ev.duration_us / 1000).toFixed(1)}ms  turns=${ev.rollout_turns} plays=${ev.rollout_plays} atks=${ev.rollout_attacks} deaths=${ev.rollout_deaths} fires=${ev.rollout_handler_fires}  winner=${ev.winner}  path=[${path}]`;
    }
    default:
      return `[${t}ms ${ev.kind || '?'}] ${JSON.stringify(ev)}`;
  }
}

// Append a TraceEvent array as formatted lines into the LOG panel.
// Trace + log share the panel; Error events get the full styled
// block via appendErrorEvent so they're impossible to miss when
// scrolling.
function appendTrace(events) {
  if (!events || events.length === 0) return;
  for (const ev of events) {
    if (ev && ev.kind === 'Error') {
      appendErrorEvent(ev);
      continue;
    }
    window.tsotLogPushText(fmtTraceEvent(ev));
  }
}

// JS-side observability summary line. The wasm-side trace stops at
// FFI exit; everything between Rust returning and the next FFI call
// is JS / DOM / browser work. parseEnvelope brackets each phase with
// `performance.now()` and emits one summary line per click into the
// LOG panel.
function jsPushSummary(label, breakdown) {
  const parts = Object.entries(breakdown)
    .filter(([_, v]) => v !== undefined)
    .map(([k, v]) => `${k}=${typeof v === 'number' ? v.toFixed(1) + 'ms' : v}`)
    .join(' ');
  window.tsotLogPushText(`[js summary] ${label}: ${parts}`);
}

// Per-item action handler for the `saved_item_action` idb op. Reads
// the save record, dispatches to Load (delegates to play.html via
// window.tsotLoadSaveJson — it mutates live game state we haven't
// ported yet), Download (transient Blob + a.click), or Delete
// (confirm + IDB delete + push refreshed list back through savedListIn).
async function tsotHandleSavedItemAction(action, id, savedListInPort) {
  const rec = await tsotDbGetSaveById(id);
  if (!rec) {
    tsotSetSaveStatus('save ' + id + ' not found');
    return;
  }
  switch (action) {
    case 'load':
      await window.tsotLoadSaveJson(rec.json);
      return;
    case 'download': {
      const blob = new Blob([rec.json], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = rec.name.replace(/[^a-z0-9_-]+/gi, '_') + '.json';
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      return;
    }
    case 'delete': {
      if (!confirm('Delete "' + rec.name + '"?')) {
        // User cancelled — re-send the same list so Elm exits its
        // `SavedLoading` state (the click moved it there before the
        // confirm dialog fired).
        const records = await tsotDbGetAllSaves();
        savedListInPort.send({ items: tsotSavesToMetadataList(records) });
        return;
      }
      await tsotDbDeleteSave(id);
      const records = await tsotDbGetAllSaves();
      savedListInPort.send({ items: tsotSavesToMetadataList(records) });
      return;
    }
    default:
      throw new Error('unknown saved_item_action: ' + String(action));
  }
}

// Errors are sacred. The js-bridge IIFE has multiple sequential blocks
// (Elm.Main.init, port subscribers, window.tsot* shim definitions); a
// throw at line N silently kills every block after it, and the failure
// surfaces N indirections later as `window.tsotXxx is not a function`
// at a *consumer* in play.html — useless. tsotShowBridgeFailure surfaces
// the throw at its origin: a fixed-position red banner pinned to the
// top of the page, carrying stage name + message + stack. No DevTools
// needed. The error names itself, visible to the developer in-place,
// the moment it happens.
function tsotShowBridgeFailure(stage, err) {
  var msg = '[js-bridge crashed at stage: ' + stage + '] '
    + (err && err.message ? err.message : String(err));
  var stack = err && err.stack ? String(err.stack) : '';
  try { console.error(msg, stack); } catch (_) { /* console may not exist */ }
  try {
    if (document.getElementById('tsot-bridge-failure')) return;
    var banner = document.createElement('div');
    banner.id = 'tsot-bridge-failure';
    banner.style.cssText =
      'position:fixed;top:0;left:0;right:0;z-index:99999;'
      + 'padding:0.5rem 0.75rem;background:#3a0a0a;'
      + 'border-bottom:2px solid #f44;'
      + 'color:#fcc;font-family:ui-monospace,Menlo,monospace;'
      + 'font-size:0.75rem;white-space:pre-wrap;word-break:break-word;'
      + 'max-height:60vh;overflow:auto;';
    var head = document.createElement('div');
    head.style.cssText = 'color:#f88;font-weight:bold;';
    head.textContent = msg;
    banner.appendChild(head);
    if (stack) {
      var pre = document.createElement('div');
      pre.style.cssText = 'color:#caa;margin-top:0.25rem;';
      pre.textContent = stack;
      banner.appendChild(pre);
    }
    document.body.insertBefore(banner, document.body.firstChild);
  } catch (_) { /* if even DOM injection fails, console.error already fired */ }
}

(function () {
  var stage = 'enter';
  try {
  stage = 'find #elm-root';
  var node = document.getElementById('elm-root');
  if (!node) {
    throw new Error('<div id="elm-root"> missing from play.html');
  }
  stage = 'check Elm.Main';
  if (typeof Elm === 'undefined' || !Elm.Main || typeof Elm.Main.init !== 'function') {
    throw new Error('Elm.Main missing — did bundle.js load before js-bridge.js?');
  }
  stage = 'Elm.Main.init';
  var app = Elm.Main.init({ node: node });

  stage = 'buildInfo port';
  // Hand the build-info envelope to Elm. `window.__TSOT_BUILD__` is set
  // by `build-info.js` (generated by `make wasm` / `make wasm-dev`) or
  // forced to `null` by the script tag's `onerror` if the file is
  // missing. Elm decodes; failure (including null) renders the
  // "build info unavailable" footer. Send is queued by Elm's runtime if
  // the subscription isn't registered yet on this tick.
  if (app && app.ports && app.ports.buildInfoIn) {
    app.ports.buildInfoIn.send(window.__TSOT_BUILD__ || null);
  } else {
    console.error('js-bridge: app.ports.buildInfoIn missing — Main.elm port wiring drift?');
  }

  stage = 'LOG ports';
  // LOG bridge — every appender in play.html's inline JS calls one of
  // these instead of mutating #log directly. Errors are sacred: the
  // shim accepts a pre-shaped object {source, message, location,
  // ffi_call, at_us, breadcrumb: [strings], js_stack, raw_stderr};
  // play.html's `appendErrorEvent` pre-formats `recent_trace` into
  // strings via `fmtTraceEvent` before calling tsotLogPushError.
  if (app && app.ports && app.ports.logTextIn && app.ports.logErrorIn) {
    window.tsotLogPushText = function (line) {
      app.ports.logTextIn.send(String(line));
    };
    window.tsotLogPushError = function (formatted) {
      app.ports.logErrorIn.send(formatted);
    };
  } else {
    console.error('js-bridge: log ports missing — Main.elm port wiring drift?');
    window.tsotLogPushText = function () {};
    window.tsotLogPushError = function () {};
  }

  stage = 'workerCmdOut dispatcher';
  // Stage 9 bridge collapse: one outbound port for every worker-bound
  // action. Elm sends a string cmd; this dispatcher routes to the
  // existing window.tsot* helpers in play.html. Unknown cmds throw
  // and surface in the red fault-surface banner (see
  // tsotShowBridgeFailure) — silent degradation no longer possible.
  if (!(app && app.ports && app.ports.workerCmdOut)) {
    throw new Error('workerCmdOut port missing — Main.elm wiring drift');
  }
  app.ports.workerCmdOut.subscribe(async function (envelope) {
    const cmd = envelope && envelope.cmd;
    const payload = envelope && envelope.payload;
    try {
      switch (cmd) {
        case 'save_game':
          await window.tsotSaveGame();
          break;
        case 'download':
          await window.tsotDownloadGame();
          break;
        case 'load_from_file': {
          const input = document.createElement('input');
          input.type = 'file';
          input.accept = 'application/json';
          input.style.display = 'none';
          input.onchange = async function (ev) {
            const file = ev.target.files && ev.target.files[0];
            try {
              if (file) {
                const text = await file.text();
                await window.tsotLoadSaveJson(text);
              }
            } catch (e) {
              tsotShowBridgeFailure('workerCmd:load_from_file', e);
            } finally {
              if (input.parentNode) input.parentNode.removeChild(input);
            }
          };
          document.body.appendChild(input);
          input.click();
          break;
        }
        case 'test_panic':
          await window.tsotTestPanic();
          break;
        case 'start_game':
          await window.tsotStartGameFromDeckbuilder(payload);
          break;
        case 'start_spectate':
          await window.tsotStartSpectate(payload);
          break;
        case 'spec_seek':
          window.tsotSpecSeek(payload && payload.index);
          break;
        case 'spec_step':
          window.tsotSpecStep(payload && payload.delta);
          break;
        case 'spec_play':
          window.tsotSpecPlay();
          break;
        case 'spec_pause':
          window.tsotSpecPause();
          break;
        case 'spec_fwd_end':
          window.tsotSpecFwdEnd();
          break;
        case 'spec_set_speed':
          window.tsotSpecSetSpeed(payload && payload.ms);
          break;
        case 'spec_exit':
          window.tsotSpecExit();
          break;
        case 'apply_action':
          await window.tsotApplyAction(payload);
          break;
        default:
          throw new Error('unknown worker cmd: ' + String(cmd));
      }
    } catch (e) {
      tsotShowBridgeFailure('workerCmd:' + String(cmd), e);
    }
  });

  stage = 'idbReqOut dispatcher';
  // Stage 9 bridge collapse: one outbound port for every IDB-bound
  // action. Elm sends `{op, payload}`; this dispatcher routes by op.
  // Unknown ops throw and surface in the red fault-surface banner.
  if (!(app && app.ports && app.ports.idbReqOut
        && app.ports.decisionLogIn && app.ports.savedListIn)) {
    throw new Error('idbReqOut / decisionLogIn / savedListIn port missing — Main.elm wiring drift');
  }
  app.ports.idbReqOut.subscribe(async function (envelope) {
    const op = envelope && envelope.op;
    const payload = envelope && envelope.payload;
    try {
      switch (op) {
        case 'decision_get_all': {
          const records = await tsotDbGetAllDecisions();
          app.ports.decisionLogIn.send(records);
          break;
        }
        case 'decision_export': {
          const records = await tsotDbGetAllDecisions();
          if (!records || records.length === 0) {
            tsotSetSaveStatus('no decisions yet');
            break;
          }
          const jsonl = records.map((r) => JSON.stringify(r)).join('\n') + '\n';
          const blob = new Blob([jsonl], { type: 'application/x-ndjson' });
          const url = URL.createObjectURL(blob);
          const a = document.createElement('a');
          a.href = url;
          a.download = `tsot-decisions-${new Date().toISOString().replace(/[:.]/g, '-')}.jsonl`;
          document.body.appendChild(a);
          a.click();
          document.body.removeChild(a);
          URL.revokeObjectURL(url);
          tsotSetSaveStatus(`exported ${records.length} record(s)`);
          break;
        }
        case 'decision_clear': {
          const records = await tsotDbGetAllDecisions();
          const n = records ? records.length : 0;
          if (n === 0) {
            tsotSetSaveStatus('no decisions to clear');
            break;
          }
          if (!confirm(`Delete all ${n} decision-log record(s)?`)) break;
          await tsotDbClearDecisions();
          tsotSetSaveStatus(`cleared ${n} record(s)`);
          break;
        }
        case 'saved_get_all': {
          const records = await tsotDbGetAllSaves();
          app.ports.savedListIn.send({ items: tsotSavesToMetadataList(records) });
          break;
        }
        case 'saved_item_action': {
          await tsotHandleSavedItemAction(
            payload && payload.action,
            payload && payload.id,
            app.ports.savedListIn
          );
          break;
        }
        default:
          throw new Error('unknown idb op: ' + String(op));
      }
    } catch (e) {
      tsotShowBridgeFailure('idbReq:' + String(op), e);
    }
  });

  stage = 'bootDataIn shim';
  // Stage 10 deckbuilder retry: play.html's bootstrap calls
  // window.tsotBootData({cardPool, presets}) once it has both from the
  // worker (list_card_pool + list_preset_decks). Elm decodes into
  // Model.cardPool / Model.presets and seeds the working deck with the
  // Starter preset on first paint.
  if (!app.ports.bootDataIn) {
    throw new Error('bootDataIn port missing — Main.elm wiring drift');
  }
  window.tsotBootData = function (data) {
    app.ports.bootDataIn.send(data);
  };

  stage = 'gameMetaIn shim';
  // Stage 11a/b: meta line from play.html's _renderInner.
  if (!app.ports.gameMetaIn) {
    throw new Error('gameMetaIn port missing — Main.elm wiring drift');
  }
  window.tsotPushGameMeta = function (envelope) {
    app.ports.gameMetaIn.send(envelope);
  };

  stage = 'promptTextIn shim';
  // Stage 11c: #prompt line. Every play.html setPrompt(text) call
  // routes here. Elm's viewPromptText is the sole renderer; the
  // original <div id="prompt">Loading...</div> is gone.
  if (!app.ports.promptTextIn) {
    throw new Error('promptTextIn port missing — Main.elm wiring drift');
  }
  window.tsotSetPrompt = function (text) {
    app.ports.promptTextIn.send(String(text));
  };

  stage = 'gameStateIn shim';
  // Stage 11d: full {state, prompt} envelope pushed from
  // play.html's _renderInner on every render. Stored raw as
  // Model.gameState — no decoder yet, subsequent substages decode
  // slices as they need them.
  if (!app.ports.gameStateIn) {
    throw new Error('gameStateIn port missing — Main.elm wiring drift');
  }
  window.tsotPushGameState = function (envelope) {
    app.ports.gameStateIn.send(envelope);
  };

  stage = 'tsotPushUctPreview shim';
  // Chunk B/C Wave 0 — UCT preview push. The JS-side preview kickoff
  // in play.html (maybeFireUctPreview) still owns cancellation +
  // worker round-trip + stale-promise guard; when it lands, the
  // result envelope flows here. Wave 5 consumes it for card-badge
  // rendering.
  if (!app.ports.uctPreviewIn) {
    throw new Error('uctPreviewIn port missing — Main.elm wiring drift');
  }
  window.tsotPushUctPreview = function (envelope) {
    app.ports.uctPreviewIn.send(envelope);
  };

  stage = 'tsotPushSpectatorState shim';
  // Stage 12 — spectator bar projection push. play.html's
  // spectateRenderCurrent / spectateExit / start-spectate path call
  // this with `{active, index, total, playing, msPerStep, winner,
  // snapshot:{turn,phase,activePlayer}|null}`. Elm's SpectatorBar
  // module is the sole renderer; the previous getElementById writes
  // on #spec-slider / #spec-readout / #spec-play are gone.
  if (!app.ports.spectatorStateIn) {
    throw new Error('spectatorStateIn port missing — Main.elm wiring drift');
  }
  window.tsotPushSpectatorState = function (envelope) {
    app.ports.spectatorStateIn.send(envelope);
  };

  // Push a fresh saves list to Elm. Used by play.html's onSaveClick
  // after a successful Save — if the panel happens to be open it
  // refreshes in place; if hidden, Elm ignores the update (see
  // `SavedListReceived`).
  window.tsotSavedListRefresh = async function () {
    try {
      const records = await tsotDbGetAllSaves();
      app.ports.savedListIn.send({ items: tsotSavesToMetadataList(records) });
    } catch (e) {
      tsotShowBridgeFailure('tsotSavedListRefresh', e);
    }
  };

  stage = 'tsotSetSaveStatus / tsotSetPhase shims';
  // tsotSetSaveStatus + tsotSetPhase — exposed to play.html so the
  // existing `setSaveStatus(msg)` + each `state.phase = '...'`
  // assignment forwards its update into Elm's model. Stage 6 makes
  // the save-status `<span>` and the Save/Download enabled-state
  // both functions of Elm's `Model.saveStatus` / `Model.gamePhase`.
  if (app && app.ports && app.ports.saveStatusIn && app.ports.gamePhaseIn) {
    window.tsotSetSaveStatus = function (msg) {
      app.ports.saveStatusIn.send(String(msg));
    };
    window.tsotSetPhase = function (phase) {
      app.ports.gamePhaseIn.send(String(phase));
    };
  } else {
    console.error('js-bridge: saveStatusIn / gamePhaseIn ports missing — Main.elm port wiring drift?');
    window.tsotSetSaveStatus = function () {};
    window.tsotSetPhase = function () {};
  }

  stage = 'ready';
  // Proof of successful run + the literal contents of `app.ports`.
  // Lands in Elm's LOG panel (visible in the page). If we see this
  // line, the IIFE ran to completion. If we don't see it, the red
  // banner above will name the stage that died.
  try {
    var portKeys = (app && app.ports) ? Object.keys(app.ports).sort().join(',') : 'NONE';
    if (typeof window.tsotLogPushText === 'function') {
      window.tsotLogPushText('[js-bridge] ready · app.ports=[' + portKeys + ']');
    }
  } catch (_) { /* logging failure must never become the bug */ }
  } catch (e) {
    tsotShowBridgeFailure(stage, e);
  }
})();
