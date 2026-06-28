// Chat overlay. Bottom-left translucent panel — scroll log + single
// input. Wasm publishes own + receives peer chat via
// `window.__raveChatRecv`; pressing Enter calls the exported wasm
// `rave_chat_send`. Focus state is pushed into wasm so player WASD is
// suppressed while the input has focus.

import { showErr } from "./overlay";

export interface ChatBridge {
  send: (body: string) => void;
  setFocus: (focused: boolean) => void;
}

interface ChatWire {
  peer: string;
  body: string;
  at_ms: number;
}

const LOG_CAP = 100;
const PEER_TAIL = 6;

function shortPeer(peer: string): string {
  return peer.length > PEER_TAIL ? "…" + peer.slice(-PEER_TAIL) : peer;
}

function appendRow(log: HTMLDivElement, wire: ChatWire): void {
  const row = document.createElement("div");
  row.className = "row";
  const peerSpan = document.createElement("span");
  peerSpan.className = "peer";
  peerSpan.textContent = shortPeer(wire.peer);
  const bodySpan = document.createElement("span");
  bodySpan.className = "body";
  bodySpan.textContent = wire.body;
  row.appendChild(peerSpan);
  row.appendChild(bodySpan);
  log.appendChild(row);
  while (log.children.length > LOG_CAP) {
    if (log.firstChild) log.removeChild(log.firstChild);
  }
  log.scrollTop = log.scrollHeight;
}

export function installChatOverlay(bridge: ChatBridge): void {
  const log = document.getElementById("rave-chat-log") as HTMLDivElement | null;
  const input = document.getElementById(
    "rave-chat-input",
  ) as HTMLInputElement | null;
  if (!log || !input) {
    showErr("[chat-overlay] DOM nodes missing — index.html out of sync");
    return;
  }

  window.__raveChatRecv = (json: string): void => {
    try {
      const wire = JSON.parse(json) as ChatWire;
      appendRow(log, wire);
    } catch (e) {
      showErr(`[__raveChatRecv parse failed] ${e}: ${json}`);
    }
  };

  input.addEventListener("focus", () => bridge.setFocus(true));
  input.addEventListener("blur", () => bridge.setFocus(false));

  input.addEventListener("keydown", (ev: KeyboardEvent) => {
    if (ev.key === "Escape") {
      input.blur();
      ev.preventDefault();
      return;
    }
    if (ev.key !== "Enter") return;
    ev.preventDefault();
    const body = input.value.trim();
    if (body.length === 0) {
      input.blur();
      return;
    }
    bridge.send(body);
    input.value = "";
  });

  // Global Enter focuses the input — lets the user start typing
  // mid-game without clicking the panel. Skipped if any other input
  // is already focused (chat input doesn't re-focus itself; other
  // forms aren't hijacked).
  window.addEventListener("keydown", (ev: KeyboardEvent) => {
    if (ev.key !== "Enter") return;
    const active = document.activeElement;
    if (active === input) return;
    const tag = active?.tagName?.toLowerCase();
    if (tag === "input" || tag === "textarea") return;
    input.focus();
    ev.preventDefault();
  });
}
