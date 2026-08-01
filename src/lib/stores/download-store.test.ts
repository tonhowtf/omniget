import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

type DownloadStore = typeof import("./download-store.svelte");

let store: DownloadStore;

const queueItem = (id: number) => ({
  id,
  url: "https://example.com/video",
  platform: "youtube",
  title: "Example video",
  status: { type: "Active" as const },
  percent: 0,
  speed_bytes_per_sec: 0,
  downloaded_bytes: 0,
  total_bytes: 100,
  file_path: null,
  file_size_bytes: null,
  file_count: null,
  thumbnail_url: null,
});

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 0;
  });
  store = await import("./download-store.svelte");
});

afterEach(() => {
  store.syncQueueState([]);
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("generic download progress", () => {
  it("does not recreate an item removed by an authoritative queue update", () => {
    const id = 901;
    store.syncQueueState([queueItem(id)]);
    store.syncQueueState([]);

    store.upsertGenericProgress(id, "Example video", "youtube", 50, 10, 50, 100, "downloading");

    expect(store.getDownloads().has(id)).toBe(false);
  });

  it("accepts progress again after the queue restores the same item", () => {
    const id = 902;
    store.syncQueueState([queueItem(id)]);
    store.syncQueueState([]);
    store.syncQueueState([queueItem(id)]);

    store.upsertGenericProgress(id, "Example video", "youtube", 50, 10, 50, 100, "downloading");

    expect(store.getDownloads().get(id)).toMatchObject({
      kind: "generic",
      percent: 50,
      downloadedBytes: 50,
      status: "downloading",
    });
  });
});
