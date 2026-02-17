const POLL_INTERVAL = 2000;
const URL_PATTERN = /^https?:\/\/.+/;

type ClipboardCallback = (url: string) => void;

let lastDetectedUrl = "";
let intervalId: ReturnType<typeof setInterval> | null = null;
let callback: ClipboardCallback | null = null;
let enabled = false;

export function onClipboardUrl(cb: ClipboardCallback | null) {
  callback = cb;
}

export function startClipboardMonitor() {
  if (intervalId) return;
  enabled = true;

  intervalId = setInterval(async () => {
    if (!enabled || !document.hasFocus()) return;

    try {
      const text = await navigator.clipboard.readText();
      const trimmed = text.trim().split(/[\s\n]/)[0];

      if (
        trimmed &&
        URL_PATTERN.test(trimmed) &&
        trimmed !== lastDetectedUrl
      ) {
        lastDetectedUrl = trimmed;
        callback?.(trimmed);
      }
    } catch {
      // clipboard read denied or unavailable
    }
  }, POLL_INTERVAL);
}

export function stopClipboardMonitor() {
  enabled = false;
  if (intervalId) {
    clearInterval(intervalId);
    intervalId = null;
  }
}

export function resetLastDetected() {
  lastDetectedUrl = "";
}
