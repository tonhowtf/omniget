import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

type DownloadStore = typeof import("./download-store.svelte");

let store: DownloadStore;

type QueueItem = {
  id: number;
  url: string;
  platform: string;
  title: string;
  status: { type: string; data?: unknown };
  percent: number | null;
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
});

beforeEach(async () => {
  vi.resetModules();
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

  it("does not let late progress resurrect a cancelled item", () => {
    const id = 903;
    store.syncQueueState([queueItem(id)]);
    store.syncQueueState([
      queueItem(id, {
        status: { type: "Error", data: { message: "Cancelled", retryable: false } },
        speed_bytes_per_sec: 0,
      }),
    ]);

    store.upsertGenericProgress(id, "Example video", "youtube", 50, 10, 50, 100, "downloading");

    expect(store.getDownloads().get(id)).toMatchObject({
      status: "error",
      speed: 0,
      error: "Cancelled",
    });
  });
});

describe("unknown total (D-04)", () => {
  it("keeps a null queue percent as null instead of a number", () => {
    store.syncQueueState([queueItem(950, { percent: null, total_bytes: null, downloaded_bytes: 2_400_000 })]);
    const item = store.getDownloads().get(950)!;
    expect(item.percent).toBeNull();
    expect(item.kind === "generic" && item.downloadedBytes).toBe(2_400_000);
  });

  it("keeps a null progress-event percent as null and the aggregate indeterminate", () => {
    store.syncQueueState([queueItem(951, { total_bytes: null })]);
    store.upsertGenericProgress(951, "Example video", "youtube", null, 10, 5_000, null, "downloading");
    expect(store.getDownloads().get(951)!.percent).toBeNull();
    expect(store.getAggregate().percent).toBeNull();
  });

  it("clamps known percents and maps non-finite ones to null", () => {
    expect(store.knownPercent(42)).toBe(42);
    expect(store.knownPercent(-3)).toBe(0);
    expect(store.knownPercent(null)).toBeNull();
    expect(store.knownPercent(Number.NaN)).toBeNull();
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
    expect(agg.etaSeconds).toBeCloseTo(0.875);
  });

  it("uses reported progress and the largest per-item ETA when a total size is unknown", () => {
    store.syncQueueState([
      queueItem(1, { percent: 20, speed_bytes_per_sec: 1000, downloaded_bytes: 200, total_bytes: 1000, eta_seconds: 4 }),
      queueItem(2, { percent: 18, speed_bytes_per_sec: 500, downloaded_bytes: 100, total_bytes: null, eta_seconds: 42 }),
    ]);

    const agg = store.getAggregate();
    expect(agg.totalBytes).toBeNull();
    expect(agg.percent).toBe(19);
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

  it("retains queued and paused progress but excludes historical completions and seeds", () => {
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
    expect(agg.activeCount).toBe(1);
    expect(agg.speedBps).toBe(1000);
    expect(agg.downloadedBytes).toBe(2498);
    expect(agg.totalBytes).toBe(20998);
    expect(agg.percent).toBeCloseTo(2498 / 20998 * 100);
    expect(agg.etaSeconds).toBeNull();
  });

  it("reports a seeding-only queue as idle so the bar does not stay pinned", () => {
    store.syncQueueState([
      queueItem(1, {
        status: { type: "Seeding" },
        percent: 100,
        speed_bytes_per_sec: 0,
        downloaded_bytes: 4096,
        total_bytes: 4096,
      }),
    ]);

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

  it("keeps counting a seeding torrent in the sidebar badge", () => {
    store.syncQueueState([
      queueItem(1, { status: { type: "Seeding" }, percent: 100, total_bytes: 4096 }),
    ]);

    const counts = store.getCounts();
    expect(counts.active).toBe(1);
    expect(counts.badge).toBe(1);
  });

  it("drops stale samples once the queue drains", () => {
    let clock = Date.now();
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => clock);
    try {
      for (let i = 0; i < 3; i++) {
        clock += 1000;
        store.syncQueueState([
          queueItem(1, { speed_bytes_per_sec: 1000, downloaded_bytes: i * 100, total_bytes: 10_000 }),
        ]);
      }
      expect(store.getAggregateSpeedHistory().length).toBeGreaterThan(0);

      clock += 1000;
      store.syncQueueState([
        queueItem(1, { status: { type: "Complete" }, percent: 100, total_bytes: 10_000 }),
      ]);
      expect(store.getAggregateSpeedHistory()).toHaveLength(0);
    } finally {
      nowSpy.mockRestore();
    }
  });

  it("uses item progress when a course makes the aggregate byte total unknowable", () => {
    store.syncQueueState([
      queueItem(1, { speed_bytes_per_sec: 1000, downloaded_bytes: 200, total_bytes: 1000, eta_seconds: 5 }),
    ]);
    store.upsertProgress(77, "Some course", 30, "Module 1", "Page 2", 4096, 10, 3, 2, 0);

    const agg = store.getAggregate();
    expect(agg.activeCount).toBe(2);
    expect(agg.totalBytes).toBeNull();
    expect(agg.percent).toBe(15);
    expect(agg.downloadedBytes).toBe(200 + 4096);
    expect(agg.etaSeconds).toBeNull();

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

describe("aggregate batch lifecycle", () => {
  it("keeps progress when every download pauses and restores speed on resume", () => {
    store.syncQueueState([queueItem(1, { downloaded_bytes: 40, speed_bytes_per_sec: 10 })]);
    store.syncQueueState([queueItem(1, { status: { type: "Paused" }, downloaded_bytes: 40 })]);
    expect(store.getAggregate()).toMatchObject({ percent: 40, downloadedBytes: 40, totalBytes: 100, speedBps: 0, etaSeconds: null, pausedCount: 1, outcome: "working" });
    store.syncQueueState([queueItem(1, { downloaded_bytes: 50, speed_bytes_per_sec: 10 })]);
    expect(store.getAggregate()).toMatchObject({ percent: 50, speedBps: 10, etaSeconds: 5 });
  });

  it("retains completed work after clearing history while another item downloads", () => {
    store.syncQueueState([queueItem(1, { downloaded_bytes: 90 }), queueItem(2, { downloaded_bytes: 20 })]);
    const before = store.getAggregate().percent!;
    store.markGenericComplete(1, true);
    expect(store.getAggregate().percent).toBeGreaterThanOrEqual(before);
    expect(store.getAggregate()).toMatchObject({ downloadedBytes: 120, totalBytes: 200, percent: 60 });
    store.clearFinished();
    expect(store.getAggregate()).toMatchObject({ downloadedBytes: 120, totalBytes: 200, percent: 60 });
  });

  it("starts a fresh batch after completion and accepts newly queued work within a batch", () => {
    store.syncQueueState([queueItem(1, { downloaded_bytes: 90 })]);
    const firstBatch = store.getAggregate().batchId;
    store.markGenericComplete(1, true);
    expect(store.getAggregate().outcome).toBe("complete");
    store.syncQueueState([queueItem(1, { status: { type: "Complete" } }), queueItem(2, { downloaded_bytes: 20 })]);
    expect(store.getAggregate()).toMatchObject({ batchId: firstBatch + 1, downloadedBytes: 20, totalBytes: 100, percent: 20 });
    store.syncQueueState([queueItem(2, { downloaded_bytes: 20 }), queueItem(3, { status: { type: "Queued" } })]);
    expect(store.getAggregate()).toMatchObject({ batchId: firstBatch + 1, totalBytes: 200, percent: 10, etaSeconds: null });
  });

  it("does not carry a historical failure into a healthy active batch", () => {
    store.syncQueueState([
      queueItem(1, { status: { type: "Error", data: "Old failure" } }),
      queueItem(2, { speed_bytes_per_sec: 10, downloaded_bytes: 25 }),
    ]);

    expect(store.getDownloads().get(1)?.status).toBe("error");
    expect(store.getDownloads().get(2)?.status).toBe("downloading");
    expect(store.getAggregate()).toMatchObject({
      outcome: "working",
      activeCount: 1,
      failedCount: 0,
      downloadedBytes: 25,
      totalBytes: 100,
      percent: 25,
    });
  });

  it("never reports successful completion when work is cancelled or fails", () => {
    store.syncQueueState([queueItem(1), queueItem(2)]);
    store.removeDownload(1);
    store.markGenericComplete(2, true);
    expect(store.getAggregate().outcome).toBe("stopped");
    store.syncQueueState([queueItem(3)]);
    store.markGenericComplete(3, false, "Network unavailable");
    expect(store.getAggregate()).toMatchObject({ outcome: "stopped", failedCount: 1 });
  });

  it("acknowledges failures without removing items and resurfaces a failed retry", () => {
    store.syncQueueState([queueItem(1)]);
    store.markGenericComplete(1, false, "Network unavailable");
    store.dismissAggregateFailures();
    expect(store.getAggregate().failedCount).toBe(0);
    expect(store.getDownloads().get(1)?.status).toBe("error");
    store.syncQueueState([queueItem(1)]);
    store.markGenericComplete(1, false, "Network unavailable");
    expect(store.getAggregate().failedCount).toBe(1);
  });

  it("shows a new failure even when a different failed item was dismissed", () => {
    store.syncQueueState([queueItem(1), queueItem(2)]);
    store.markGenericComplete(1, false);
    store.dismissAggregateFailures();
    store.markGenericComplete(2, false);
    expect(store.getAggregate().failedCount).toBe(1);
  });

  it("retains a torrent's completed bytes when it starts seeding", () => {
    store.syncQueueState([queueItem(1, { downloaded_bytes: 90 }), queueItem(2, { downloaded_bytes: 20 })]);
    store.syncQueueState([queueItem(1, { status: { type: "Seeding" }, downloaded_bytes: 100 }), queueItem(2, { downloaded_bytes: 20 })]);
    expect(store.getAggregate()).toMatchObject({ activeCount: 1, downloadedBytes: 120, totalBytes: 200 });
    store.markGenericComplete(2, true);
    expect(store.getAggregate().outcome).toBe("complete");
  });

  it("does not estimate a partial ETA for missing or invalid estimates", () => {
    for (const eta of [null, NaN, Infinity, -1, 0]) {
      store.syncQueueState([
        queueItem(1, { speed_bytes_per_sec: 10, eta_seconds: 5 }),
        queueItem(2, { speed_bytes_per_sec: 10, total_bytes: null, eta_seconds: eta }),
      ]);
      expect(store.getAggregate().etaSeconds).toBeNull();
    }
  });

  it("hides ETA for stalled work even while another transfer is flowing", () => {
    store.syncQueueState([queueItem(1, { speed_bytes_per_sec: 10 }), queueItem(2)]);
    expect(store.getAggregate().etaSeconds).toBeNull();
  });

  it("distinguishes a mixed paused and queued state", () => {
    store.syncQueueState([queueItem(1, { status: { type: "Paused" }, downloaded_bytes: 20 }), queueItem(2, { status: { type: "Queued" } })]);
    expect(store.getAggregate()).toMatchObject({ activeCount: 0, queuedCount: 1, pausedCount: 1, downloadedBytes: 20, percent: 10, etaSeconds: null });
  });

  it("keeps unknown-sized byte counts and reported progress when paused", () => {
    store.syncQueueState([queueItem(1, { percent: 18, downloaded_bytes: 40, total_bytes: null })]);
    store.syncQueueState([queueItem(1, { status: { type: "Paused" }, percent: 18, downloaded_bytes: 40, total_bytes: null })]);
    expect(store.getAggregate()).toMatchObject({ downloadedBytes: 40, percent: 18 });
    store.markGenericComplete(1, true);
    expect(store.getAggregate()).toMatchObject({ downloadedBytes: 40, totalBytes: 40, percent: 100, outcome: "complete" });
  });
});
