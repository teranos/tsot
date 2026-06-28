// Streaming wasm fetch with real progress. Replaces the black
// screen during boot with a percent + MB indicator. The bytes are
// reassembled and handed to wasm-bindgen's init() so the boot path
// doesn't double-fetch.

const loadingText = document.getElementById(
  "rave-loading-text",
) as HTMLDivElement | null;
const loadingBar = document.querySelector(
  "#rave-loading progress",
) as HTMLProgressElement | null;

function fmtMB(bytes: number): string {
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

function updateProgress(loaded: number, total: number): void {
  if (!loadingText || !loadingBar) return;
  if (total > 0) {
    const pct = Math.floor((loaded / total) * 100);
    loadingBar.value = loaded;
    loadingBar.max = total;
    loadingText.textContent = `loading rave · ${pct}% · ${fmtMB(loaded)}/${fmtMB(total)}`;
  } else {
    loadingText.textContent = `loading rave · ${fmtMB(loaded)}`;
  }
}

export async function streamWasmBytes(url: string): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`fetch ${url} → HTTP ${response.status}`);
  }
  const total = parseInt(response.headers.get("content-length") ?? "0", 10);
  const body = response.body;
  if (!body) {
    throw new Error(`fetch ${url} → response.body missing`);
  }
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let loaded = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (value) {
      chunks.push(value);
      loaded += value.length;
      updateProgress(loaded, total);
    }
  }
  const bytes = new Uint8Array(loaded);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return bytes;
}

export function hideLoadingIndicator(): void {
  document.body.classList.add("loaded");
}
