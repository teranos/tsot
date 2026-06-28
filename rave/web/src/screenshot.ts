// Canvas → clipboard PNG. Wasm fires this on the `P` keypress.
// Every failure routes through showErr so the user sees what went
// wrong instead of a silent no-op.

import { showErr } from "./overlay";

export function installScreenshotBridge(): void {
  window.__raveScreenshot = (_filename: string): void => {
    const canvas = document.getElementById("bevy") as HTMLCanvasElement | null;
    if (!canvas) {
      showErr("[screenshot] #bevy canvas not found");
      return;
    }
    if (!navigator.clipboard || !window.ClipboardItem) {
      showErr("[screenshot] clipboard API unavailable in this browser");
      return;
    }
    canvas.toBlob((blob) => {
      if (!blob) {
        showErr("[screenshot] canvas.toBlob returned null");
        return;
      }
      navigator.clipboard
        .write([new ClipboardItem({ "image/png": blob })])
        .catch((err: unknown) => {
          showErr(`[screenshot] clipboard.write failed: ${err}`);
        });
    }, "image/png");
  };
}
