import { describe, expect, it } from "vitest";
import { formatBytes, formatElapsed, formatTimecode, parseTimecode } from "./timecode";
import {
  hasOverlaps,
  keyframeAtOrBefore,
  markIn,
  markOut,
  mergeOverlaps,
  move,
  nearestKeyframe,
  remove,
  segmentAt,
  snap,
  stepKeyframe,
  toRequestSpans,
  totalLength,
  updateSegment,
  type Segment,
} from "./segments";
import { buildOptions, checkRegion, errorKey, fileName, folderOf, isPermission, parseError, pickDefaultMic } from "./record";
import type { Sources } from "./types";

const seg = (id: string, start: number, end: number): Segment => ({ id, start, end });

describe("timecode", () => {
  it("formats with and without hours", () => {
    expect(formatTimecode(0)).toBe("00:00.000");
    expect(formatTimecode(62.5)).toBe("01:02.500");
    expect(formatTimecode(3723.25)).toBe("1:02:03.250");
    expect(formatTimecode(59.9996)).toBe("01:00.000");
    expect(formatTimecode(-3)).toBe("00:00.000");
    expect(formatTimecode(Number.NaN, false)).toBe("00:00");
  });

  it("parses common shapes", () => {
    expect(parseTimecode("62.5")).toBe(62.5);
    expect(parseTimecode("1:02.5")).toBe(62.5);
    expect(parseTimecode("01:02,250")).toBeCloseTo(62.25);
    expect(parseTimecode("1:02:03")).toBe(3723);
    expect(parseTimecode(" ")).toBeNull();
    expect(parseTimecode("1:75")).toBeNull();
    expect(parseTimecode("1:60:00")).toBeNull();
    expect(parseTimecode("a:10")).toBeNull();
    expect(parseTimecode("1:2:3:4")).toBeNull();
  });

  it("round-trips", () => {
    for (const t of [0, 1.234, 59.999, 61, 3600.5]) expect(parseTimecode(formatTimecode(t))).toBeCloseTo(t, 3);
  });

  it("formats elapsed and bytes", () => {
    expect(formatElapsed(0)).toBe("00:00");
    expect(formatElapsed(65_900)).toBe("01:05");
    expect(formatElapsed(3_725_000)).toBe("1:02:05");
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(250 * 1024 * 1024)).toBe("250 MB");
  });
});

describe("keyframes", () => {
  const kf = [0, 2, 4, 6.5];
  it("finds nearest and before", () => {
    expect(nearestKeyframe(2.9, kf)).toBe(2);
    expect(nearestKeyframe(3.1, kf)).toBe(4);
    expect(nearestKeyframe(3, kf)).toBe(2);
    expect(nearestKeyframe(99, kf)).toBe(6.5);
    expect(nearestKeyframe(-1, kf)).toBe(0);
    expect(nearestKeyframe(5, [])).toBe(5);
    expect(keyframeAtOrBefore(3.9, kf)).toBe(2);
    expect(keyframeAtOrBefore(4, kf)).toBe(4);
    expect(keyframeAtOrBefore(1, [])).toBeNull();
  });
  it("snaps only when enabled and steps", () => {
    expect(snap(3.4, kf, false)).toBe(3.4);
    expect(snap(3.4, kf, true)).toBe(4);
    expect(stepKeyframe(2, kf, 1)).toBe(4);
    expect(stepKeyframe(2, kf, -1)).toBe(0);
    expect(stepKeyframe(6.5, kf, 1)).toBeNull();
    expect(stepKeyframe(0, kf, -1)).toBeNull();
  });
});

describe("segments", () => {
  let n = 0;
  const id = () => `s${++n}`;

  it("marks in then out", () => {
    const a = markIn({ segments: [], pendingIn: null }, 2);
    const r = markOut(a, 5, id);
    expect(r.error).toBeNull();
    expect(r.marks.segments.map((s) => [s.start, s.end])).toEqual([[2, 5]]);
    expect(r.marks.pendingIn).toBeNull();
  });

  it("out without in continues from the last end", () => {
    const first = markOut({ segments: [], pendingIn: null }, 3, id);
    expect(first.added).toMatchObject({ start: 0, end: 3 });
    const second = markOut(first.marks, 7, id);
    expect(second.added).toMatchObject({ start: 3, end: 7 });
  });

  it("rejects out before in and tiny segments", () => {
    expect(markOut({ segments: [], pendingIn: 5 }, 4, id).error).toBe("before_in");
    expect(markOut({ segments: [], pendingIn: 5 }, 5.01, id).error).toBe("too_short");
  });

  it("updates within bounds", () => {
    const list = [seg("a", 1, 3)];
    expect(updateSegment(list, "a", { end: 99 }, 10)[0]).toMatchObject({ start: 1, end: 10 });
    expect(updateSegment(list, "a", { start: 3 }, 10)[0]).toMatchObject({ start: 1, end: 3 });
  });

  it("reorders and removes", () => {
    const list = [seg("a", 0, 1), seg("b", 1, 2), seg("c", 2, 3)];
    expect(move(list, 0, 2).map((s) => s.id)).toEqual(["b", "c", "a"]);
    expect(move(list, 0, 5)).toBe(list);
    expect(remove(list, "b").map((s) => s.id)).toEqual(["a", "c"]);
  });

  it("detects and merges overlaps", () => {
    const list = [seg("a", 5, 8), seg("b", 0, 2), seg("c", 1.5, 3), seg("d", 8, 9)];
    expect(hasOverlaps(list)).toBe(true);
    expect(hasOverlaps([seg("x", 0, 1), seg("y", 1, 2)])).toBe(false);
    const merged = mergeOverlaps(list);
    expect(merged.map((s) => [s.start, s.end])).toEqual([
      [0, 3],
      [5, 9],
    ]);
    expect(totalLength(merged)).toBe(7);
  });

  it("finds segment at time and builds request spans", () => {
    const list = [seg("a", 1, 2), seg("b", 4, 12)];
    expect(segmentAt(list, 1.5)).toBe(0);
    expect(segmentAt(list, 3)).toBe(-1);
    expect(toRequestSpans([...list, seg("z", 11, 11.01)], 10)).toEqual([
      { start: 1, end: 2 },
      { start: 4, end: 10 },
    ]);
  });
});

describe("record options", () => {
  const base = {
    screen: "2",
    useRegion: false,
    region: { x: "", y: "", w: "", h: "" },
    fps: 30,
    micOn: true,
    mic: "1",
    cursor: true,
    container: "mp4" as const,
    quality: "balanced" as const,
    outDir: " ",
  };

  it("builds valid options", () => {
    const { options, problem } = buildOptions(base);
    expect(problem).toBeNull();
    expect(options).toMatchObject({ screen: "2", mic: "1", region: null, out_dir: null });
    expect(buildOptions({ ...base, micOn: false }).options?.mic).toBeNull();
  });

  it("reports what is missing", () => {
    expect(buildOptions({ ...base, screen: "" }).problem).toBe("no_screen");
    expect(buildOptions({ ...base, mic: null }).problem).toBe("no_mic");
    expect(buildOptions({ ...base, useRegion: true }).problem).toBe("incomplete");
  });

  it("checks regions", () => {
    expect(checkRegion({ x: "0", y: "10", w: "1280", h: "720" }).region).toEqual({ x: 0, y: 10, w: 1280, h: 720 });
    expect(checkRegion({ x: "0", y: "1.5", w: "20", h: "20" }).problem).toBe("not_integer");
    expect(checkRegion({ x: "0", y: "0", w: "8", h: "20" }).problem).toBe("too_small");
  });

  it("prefers a real microphone", () => {
    const s: Sources = {
      platform: "macos",
      ffmpeg: true,
      screens: [],
      audio: [
        { id: "0", name: "Microsoft Teams Audio", kind: "audio" },
        { id: "1", name: "MacBook Air Microphone", kind: "audio" },
      ],
      error: null,
    };
    expect(pickDefaultMic(s)).toBe("1");
    expect(pickDefaultMic(null)).toBeNull();
    expect(pickDefaultMic({ ...s, audio: [s.audio[0]] })).toBe("0");
  });
});

describe("errors and paths", () => {
  it("parses coded errors", () => {
    expect(parseError("CAPTURE_PERMISSION_SCREEN: Input/output error")).toEqual({ code: "CAPTURE_PERMISSION_SCREEN", detail: "Input/output error" });
    expect(parseError("CAPTURE_CANCELLED")).toEqual({ code: "CAPTURE_CANCELLED", detail: "" });
    expect(parseError("CAPTURE_MADE_UP: x").code).toBe("UNKNOWN");
    expect(parseError("boom").detail).toBe("boom");
    expect(parseError(null).code).toBe("UNKNOWN");
    expect(errorKey("CAPTURE_PERMISSION_MIC")).toBe("capture.err_permission_mic");
    expect(errorKey("UNKNOWN")).toBe("capture.err_unknown");
    expect(isPermission("CAPTURE_PERMISSION_MIC")).toBe(true);
    expect(isPermission("CAPTURE_RECORD_FAILED")).toBe(false);
  });

  it("splits paths on both separators", () => {
    expect(fileName("/a/b/c.mp4")).toBe("c.mp4");
    expect(fileName("C:\\v\\d.mkv")).toBe("d.mkv");
    expect(folderOf("/a/b/c.mp4")).toBe("/a/b");
    expect(folderOf("C:\\v\\d.mkv")).toBe("C:\\v");
  });
});
