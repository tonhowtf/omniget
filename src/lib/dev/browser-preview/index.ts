import { mockConvertFileSrc, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import inferred from "virtual:omniget-preview-defaults";
import { fixtures } from "./fixtures";

const missing = new Map<string, number>();
let pill: HTMLButtonElement | null = null;
let list: HTMLPreElement | null = null;

function render() {
  if (!pill) return;
  pill.textContent = missing.size === 0 ? "Preview: no backend" : `Preview: no backend · ${missing.size} without data`;
  if (list) list.textContent = [...missing.keys()].sort().join("\n");
}

function mountPill() {
  pill = document.createElement("button");
  pill.type = "button";
  pill.setAttribute("data-browser-preview", "");
  pill.style.cssText =
    "position:fixed;inset-block-end:6px;left:50%;transform:translateX(-50%);z-index:100000;padding:3px 9px;border:none;border-radius:999px;background:#1d1d1f;color:#f5f5f7;font:500 11px/1.5 system-ui;opacity:.85;cursor:pointer";
  pill.addEventListener("click", () => {
    if (list) {
      list.remove();
      list = null;
      return;
    }
    list = document.createElement("pre");
    list.style.cssText =
      "position:fixed;inset-block-end:34px;left:50%;transform:translateX(-50%);z-index:100000;max-height:60vh;overflow:auto;margin:0;padding:8px 10px;border-radius:8px;background:#1d1d1f;color:#f5f5f7;font:11px/1.5 ui-monospace,monospace";
    document.body.append(list);
    render();
  });
  document.body.append(pill);
  render();
}

export function installBrowserPreview() {
  mockWindows("main");
  mockConvertFileSrc("macos");
  mockIPC(
    (command, args) => {
      const fixture = fixtures[command];
      if (fixture) return fixture((args ?? {}) as Record<string, unknown>);
      if (command in inferred) return inferred[command];
      if (/^plugin:(window|webview)\|/.test(command)) return null;
      if (!missing.has(command)) console.warn(`[preview] no data for ${command}`, args);
      missing.set(command, (missing.get(command) ?? 0) + 1);
      render();
      throw new Error(`Preview has no data for ${command}`);
    },
    { shouldMockEvents: true },
  );
  (window as unknown as { __omnigetPreviewMissing: typeof missing }).__omnigetPreviewMissing = missing;
  if (document.body) mountPill();
  else addEventListener("DOMContentLoaded", mountPill, { once: true });
}
