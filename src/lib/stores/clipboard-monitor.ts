import { readText } from "@tauri-apps/plugin-clipboard-manager";

const POLL_INTERVAL = 2000;
const URL_PATTERN = /^(https?:\/\/|magnet:\?|p2p:).+/;

type ClipboardCallback = (url: string) => void;

let lastDetectedUrl = "";
let intervalId: ReturnType<typeof setInterval> | null = null;
const handlers: { cb: ClipboardCallback }[] = [];
let enabled = false;

export function onClipboardUrl(cb: ClipboardCallback): () => void {
  const entry = { cb };
  handlers.push(entry);
  return () => {
    const index = handlers.indexOf(entry);
    if (index !== -1) handlers.splice(index, 1);
  };
}

export function startClipboardMonitor() {
  if (intervalId) return;
  enabled = true;

  intervalId = setInterval(async () => {
    if (!enabled || !document.hasFocus()) return;

    try {
      const text = await readText();
      const trimmed = text.trim().split(/[\s\n]/)[0];

      const handler = handlers[handlers.length - 1];
      if (
        handler &&
        trimmed &&
        URL_PATTERN.test(trimmed) &&
        trimmed !== lastDetectedUrl
      ) {
        lastDetectedUrl = trimmed;
        handler.cb(trimmed);
      }
    } catch {}
  }, POLL_INTERVAL);
}

export function stopClipboardMonitor() {
  enabled = false;
  if (intervalId) {
    clearInterval(intervalId);
    intervalId = null;
  }
}
