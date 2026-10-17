import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const clipboard = vi.hoisted(() => ({ text: "" }));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: async () => clipboard.text,
}));

import { onClipboardUrl, startClipboardMonitor, stopClipboardMonitor } from "./clipboard-monitor";

async function copy(text: string) {
  clipboard.text = text;
  await vi.advanceTimersByTimeAsync(2000);
}

describe("clipboard monitor", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("document", { hasFocus: () => true });
    startClipboardMonitor();
  });

  afterEach(() => {
    stopClipboardMonitor();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("restores the previous handler when the newer one unsubscribes", async () => {
    const global = vi.fn();
    const home = vi.fn();
    const offGlobal = onClipboardUrl(global);
    const offHome = onClipboardUrl(home);

    await copy("https://example.com/a");
    expect(home).toHaveBeenCalledWith("https://example.com/a");
    expect(global).not.toHaveBeenCalled();

    offHome();
    await copy("https://example.com/b");
    expect(global).toHaveBeenCalledWith("https://example.com/b");
    expect(home).toHaveBeenCalledTimes(1);

    offGlobal();
  });

  it("keeps a URL copied while no handler is registered", async () => {
    await copy("https://example.com/c");
    const handler = vi.fn();
    const off = onClipboardUrl(handler);
    await vi.advanceTimersByTimeAsync(2000);
    expect(handler).toHaveBeenCalledWith("https://example.com/c");
    off();
  });

  it("removes only its own handler even when unsubscribed out of order", async () => {
    const first = vi.fn();
    const second = vi.fn();
    const offFirst = onClipboardUrl(first);
    const offSecond = onClipboardUrl(second);

    offFirst();
    await copy("https://example.com/d");
    expect(second).toHaveBeenCalledWith("https://example.com/d");
    expect(first).not.toHaveBeenCalled();

    offSecond();
  });
});
