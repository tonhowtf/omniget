import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

type DownloadStore = typeof import("./download-store.svelte");

let store: DownloadStore;

type QueueItem = {
  id: number;
  url: string;
  platform: string;
  title: string;
  status: { type: string; data?: unknown };
  percent: number;
  speed_bytes_per_sec: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  file_path: string | null;
  file_size_bytes: number | null;
  file_count: number | null;
  thumbnail_url: string | null;
  eta_seconds?: number | null;
};

const queueItem = (id: number, overrides: Partial<QueueItem> = {}): QueueItem => ({
  id,
  url: "https://example.com/video",
  platform: "youtube",
  title: "Example video",
  status: { type: "Active" },
  percent: 0,
  speed_bytes_per_sec: 0,
  downloaded_bytes: 0,
  total_bytes: 100,
  file_path: null,
  file_size_bytes: null,
  file_count: null,
  thumbnail_url: null,
  eta_seconds: null,
  ...overrides,
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

describe("getAggregate", () => {
  it("reports an empty queue as idle with nothing knowable", () => {
    expect(store.getAggregate()).toMatchObject({
      activeCount: 0,
      queuedCount: 0,
      pausedCount: 0,
      speedBps: 0,
      downloadedBytes: 0,
      totalBytes: null,
      percent: null,
      etaSeconds: null,
    });
  });

  it("sums speed and bytes across downloading items with known totals", () => {
    store.syncQueueState([
      queueItem(1, { speed_bytes_per_sec: 1000, downloaded_bytes: 200, total_bytes: 1000 }),
      queueItem(2, { speed_bytes_per_sec: 3000, downloaded_bytes: 300, total_bytes: 3000 }),
    ]);

    const agg = store.getAggregate();
    expect(agg.activeCount).toBe(2);
    expect(agg.speedBps).toBe(4000);
    expect(agg.downloadedBytes).toBe(500);
    expect(agg.totalBytes).toBe(4000);
    expect(agg.percent).toBeCloseTo(12.5);
    // (4000 - 500) / 4000
    expect(agg.etaSeconds).toBeCloseTo(0.875);
  });

  it("falls back to the largest per-item ETA when a total size is unknown", () => {
    store.syncQueueState([
      queueItem(1, { speed_bytes_per_sec: 1000, downloaded_bytes: 200, total_bytes: 1000, eta_seconds: 4 }),
      queueItem(2, { speed_bytes_per_sec: 500, downloaded_bytes: 100, total_bytes: null, eta_seconds: 42 }),
    ]);

    const agg = store.getAggregate();
    expect(agg.totalBytes).toBeNull();
    expect(agg.percent).toBeNull();
    expect(agg.speedBps).toBe(1500);
    expect(agg.etaSeconds).toBe(42);
  });

  it("never emits Infinity or NaN when everything is stalled", () => {
    store.syncQueueState([
      queueItem(1, { speed_bytes_per_sec: 0, downloaded_bytes: 200, total_bytes: 1000 }),
    ]);

    const agg = store.getAggregate();
    expect(agg.speedBps).toBe(0);
    expect(agg.percent).toBeCloseTo(20);
    expect(agg.etaSeconds).toBeNull();
  });

  it("excludes queued, paused, complete and seeding items from the maths", () => {
    store.syncQueueState([
      queueItem(1, { status: { type: "Active" }, speed_bytes_per_sec: 1000, downloaded_bytes: 500, total_bytes: 1000 }),
      queueItem(2, { status: { type: "Queued" }, speed_bytes_per_sec: 9000, downloaded_bytes: 999, total_bytes: 9999 }),
      queueItem(3, { status: { type: "Paused" }, speed_bytes_per_sec: 9000, downloaded_bytes: 999, total_bytes: 9999 }),
      queueItem(4, { status: { type: "Complete" }, speed_bytes_per_sec: 9000, downloaded_bytes: 999, total_bytes: 9999 }),
      queueItem(5, { status: { type: "Seeding" }, speed_bytes_per_sec: 9000, downloaded_bytes: 999, total_bytes: 9999 }),
    ]);

    const agg = store.getAggregate();
    expect(agg.queuedCount).toBe(1);
    expect(agg.pausedCount).toBe(1);
    // downloading + seeding
    expect(agg.activeCount).toBe(2);
    expect(agg.speedBps).toBe(1000);
    expect(agg.downloadedBytes).toBe(500);
    expect(agg.totalBytes).toBe(1000);
    expect(agg.percent).toBeCloseTo(50);
  });

  it("lets a course item contribute speed but forces percent to null", () => {
    store.syncQueueState([
      queueItem(1, { speed_bytes_per_sec: 1000, downloaded_bytes: 200, total_bytes: 1000, eta_seconds: 5 }),
    ]);
    store.upsertProgress(77, "Some course", 30, "Module 1", "Page 2", 4096, 10, 3, 2, 0);

    const agg = store.getAggregate();
    expect(agg.activeCount).toBe(2);
    expect(agg.totalBytes).toBeNull();
    expect(agg.percent).toBeNull();
    expect(agg.downloadedBytes).toBe(200 + 4096);
    expect(agg.etaSeconds).toBe(5);

    store.removeDownload(77);
  });

  it("caps the aggregate speed history at the history limit", () => {
    let clock = Date.now();
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => clock);
    try {
      for (let i = 0; i < 80; i++) {
        clock += 1000;
        store.syncQueueState([
          queueItem(1, { speed_bytes_per_sec: 1000 + i, downloaded_bytes: i, total_bytes: 10_000 }),
        ]);
      }
      expect(store.getAggregateSpeedHistory()).toHaveLength(60);
    } finally {
      nowSpy.mockRestore();
    }
  });
});
