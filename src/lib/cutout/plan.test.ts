import { describe, expect, it } from "vitest";
import {
  addClick,
  addMoment,
  clampFrame,
  formatMoment,
  installPercent,
  MAX_CLICKS,
  modelBytes,
  modelReady,
  moveFrame,
  newRows,
  pending,
  pointIn,
  REFINE_MODEL,
  rowId,
  tally,
  toItems,
  undoClick,
  type Row,
} from "./plan";
import type { Inspected, Status } from "./types";

function picture(path: string): Inspected {
  return { path, name: path.split("/").pop()!, info: { path, name: "", kind: "image", width: 800, height: 600, duration_ms: 0 }, error: null };
}

function video(path: string, duration: number): Inspected {
  return { path, name: path.split("/").pop()!, info: { path, name: "", kind: "video", width: 1920, height: 1080, duration_ms: duration }, error: null };
}

function broken(path: string): Inspected {
  return { path, name: path, info: null, error: "The file is empty (0 bytes)." };
}

const STATUS: Status = {
  runtime_installed: true,
  runtime_downloadable: true,
  models: [
    { id: "u2netp", name: "U²-Net light", bytes: 4_574_861, license: "Apache-2.0", source: "", downloaded: true },
    { id: "silueta", name: "Silueta", bytes: 44_173_029, license: "Apache-2.0", source: "", downloaded: false },
  ],
  refine: { id: REFINE_MODEL, name: "SAM 2.1 tiny", bytes: 130_129_714, license: "Apache-2.0", source: "", downloaded: false },
};

describe("rows", () => {
  it("adds pictures once and starts videos a little after the beginning", () => {
    const rows = newRows([picture("/a.png"), picture("/a.png"), video("/clip.mp4", 120_000), broken("/x.png")], []);
    expect(rows.map((r) => r.id)).toEqual(["/a.png", "/clip.mp4#5000", "/x.png"]);
    expect(rows[1].frameMs).toBe(5000);
    expect(newRows([picture("/a.png")], ["/a.png"])).toEqual([]);
    expect(newRows([video("/short.mp4", 2000)], [])[0].frameMs).toBe(200);
  });

  it("keeps frames inside the video", () => {
    expect(clampFrame(-5, 10_000)).toBe(0);
    expect(clampFrame(Number.NaN, 10_000)).toBe(0);
    expect(clampFrame(99_999, 10_000)).toBe(9900);
    expect(clampFrame(1234.6, 10_000)).toBe(1235);
  });

  it("moves a frame and drops its clicks, unless another row already has that moment", () => {
    const rows = newRows([video("/v.mp4", 60_000)], []);
    rows[0].clicks = [{ x: 0.5, y: 0.5, keep: true }];
    const moved = moveFrame(rows, rows[0], 30_000)!;
    expect(moved.id).toBe(rowId("/v.mp4", 30_000));
    expect(moved.clicks).toEqual([]);
    const two = addMoment(rows, rows[0]);
    expect(moveFrame(two, two[0], two[1].frameMs!)).toBeNull();
  });

  it("adds another moment right after the video it came from", () => {
    const rows = newRows([video("/v.mp4", 60_000), picture("/p.png")], []);
    const more = addMoment(rows, rows[0]);
    expect(more.map((r) => r.id)).toEqual(["/v.mp4#5000", "/v.mp4#11000", "/p.png"]);
    const again = addMoment(more, more[0]);
    expect(again[1].id).toBe("/v.mp4#12000");
  });

  it("sends every row with its moment and clicks", () => {
    const rows = newRows([picture("/a.png"), video("/v.mp4", 60_000)], []);
    rows[0].clicks = [{ x: 0.1, y: 0.2, keep: false }];
    expect(toItems(rows)).toEqual([
      { path: "/a.png", frame_ms: null, clicks: [{ x: 0.1, y: 0.2, keep: false }] },
      { path: "/v.mp4", frame_ms: 5000, clicks: [] },
    ]);
  });

  it("counts what is left and how it went", () => {
    const rows: Row[] = newRows([picture("/a.png"), picture("/b.png"), broken("/c.png")], []);
    rows[0].status = "done";
    expect(pending(rows).map((r) => r.id)).toEqual(["/b.png"]);
    rows[1].status = "failed";
    expect(tally(rows)).toMatchObject({ done: 1, failed: 1, idle: 1 });
  });
});

describe("clicks", () => {
  it("maps a pointer to the picture and ignores clicks outside it", () => {
    const rect = { left: 100, top: 50, width: 200, height: 100 };
    expect(pointIn(200, 100, rect)).toEqual({ x: 0.5, y: 0.5 });
    expect(pointIn(99, 100, rect)).toBeNull();
    expect(pointIn(200, 151, rect)).toBeNull();
    expect(pointIn(0, 0, { left: 0, top: 0, width: 0, height: 0 })).toBeNull();
  });

  it("keeps the latest clicks and undoes the last one", () => {
    let clicks = [{ x: 0, y: 0, keep: true }];
    for (let i = 0; i < MAX_CLICKS + 5; i++) clicks = addClick(clicks, { x: i, y: i, keep: i % 2 === 0 });
    expect(clicks).toHaveLength(MAX_CLICKS);
    expect(clicks[clicks.length - 1].x).toBe(MAX_CLICKS + 4);
    expect(undoClick(clicks)).toHaveLength(MAX_CLICKS - 1);
    expect(undoClick([])).toEqual([]);
  });
});

describe("models", () => {
  it("knows what is ready and what still has to be downloaded", () => {
    expect(modelReady(STATUS, "u2netp")).toBe(true);
    expect(modelReady(STATUS, "silueta")).toBe(false);
    expect(modelReady(STATUS, REFINE_MODEL)).toBe(false);
    expect(modelReady({ ...STATUS, runtime_installed: false }, "u2netp")).toBe(false);
    expect(modelReady(null, "u2netp")).toBe(false);
    expect(modelBytes(STATUS, "silueta")).toBe(44_173_029);
    expect(modelBytes(STATUS, REFINE_MODEL)).toBe(130_129_714);
    expect(modelBytes(STATUS, "nope")).toBe(0);
  });

  it("adds up the download across the runtime and every file", () => {
    expect(installPercent({})).toBeNull();
    expect(installPercent({ a: { done: 5, total: null } })).toBeNull();
    expect(installPercent({ runtime: { done: 30, total: 30 }, model: { done: 10, total: 70 } })).toBe(40);
    expect(installPercent({ a: { done: 200, total: 100 } })).toBe(100);
  });
});

describe("labels", () => {
  it("writes moments the short way", () => {
    expect(formatMoment(0)).toBe("0:00");
    expect(formatMoment(83_500)).toBe("1:23");
    expect(formatMoment(3_723_000)).toBe("1:02:03");
  });
});
