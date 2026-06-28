// Typed declarations for every window-level extern the wasm side
// reaches into. Declared once so a bridge signature drift surfaces
// at the TypeScript type-check, not at runtime as a "not a function".

import type { TypedError } from "./error-bridge";

declare global {
  interface Window {
    __raveError: (line: string) => void;
    __raveErrorTyped: (json: string) => void;
    __raveLoadIdentity: () => Promise<Uint8Array | null>;
    __raveSaveIdentity: (bytes: Uint8Array) => Promise<void>;
    __raveScreenshot: (filename: string) => void;
    __raveChatRecv: (json: string) => void;
  }
}

export {};

// Re-export for any module that wants the structural shape.
export type { TypedError };
