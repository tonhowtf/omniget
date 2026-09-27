import { describe, expect, it } from "vitest";
import {
  comparable,
  formatClock,
  formatDelta,
  formatFps,
  formatMetric,
  frametimeAxis,
  histogramBars,
  nearest,
  niceMax,
  seriesPaths,
  smoothness,
  timeX,
  type SessionSummary,
} from "./perf";

describe("perf formatting", () => {
  it("formats fps, clock and metrics", () => {
    expect(formatFps(143.6)).toBe("144");
    expect(formatFps(59.94)).toBe("59.9");
    expect(formatFps(null)).toBe("—");
    expect(formatClock(65)).toBe("1:05");
    expect(formatClock(3723)).toBe("1:02:03");
    expect(formatMetric("frametime_p99_ms", 21.345)).toBe("21.3 ms");
    expect(formatMetric("stutter_time_pct", 2.25)).toBe("2.3%");
    expect(formatMetric("low_1", 88.2)).toBe("88.2");
  });

  it("formats deltas with sign and unit", () => {
    expect(formatDelta({ metric: "fps_avg", delta_pct: 10.24, verdict: "better" })).toBe("+10.2%");
    expect(formatDelta({ metric: "fps_avg", delta_pct: -3, verdict: "worse" })).toBe("−3.0%");
    expect(formatDelta({ metric: "stutter_time_pct", delta_pct: 1.2, verdict: "worse" })).toBe("+1.2 pt");
    expect(formatDelta({ metric: "fps_avg", delta_pct: 0.01, verdict: "same" })).toBe("0%");
    expect(formatDelta({ metric: "fps_avg", delta_pct: null, verdict: "unknown" })).toBe("—");
  });

  it("rates smoothness by 1% low over average", () => {
    expect(smoothness(100, 80)).toBe("good");
    expect(smoothness(100, 60)).toBe("warn");
    expect(smoothness(100, 30)).toBe("bad");
    expect(smoothness(null, 30)).toBe("default");
  });
});

describe("perf charts", () => {
  it("rounds axis maxima", () => {
    expect(niceMax(17)).toBe(20);
    expect(niceMax(21)).toBe(25);
    expect(niceMax(0.3)).toBe(0.5);
    expect(niceMax(0)).toBe(1);
  });

  it("lays out histogram bars with overflow", () => {
    const bars = histogramBars({ start_ms: 6, width_ms: 1, counts: [0, 10, 5], overflow: 2 }, 400, 100);
    expect(bars).toHaveLength(4);
    expect(bars[0].h).toBe(0);
    expect(bars[1].h).toBe(100);
    expect(bars[1].from).toBe(7);
    expect(bars[3].overflow).toBe(true);
    expect(bars[3].to).toBe(Infinity);
    expect(histogramBars({ start_ms: 0, width_ms: 1, counts: [], overflow: 0 }, 100, 100)).toEqual([]);
  });

  it("builds series paths and finds points", () => {
    const s: [number, number, number][] = [
      [0, 10, 12],
      [500, 10, 40],
      [1000, 9, 11],
    ];
    const p = seriesPaths(s, 100, 50, 50);
    expect(p.line.startsWith("M0.0,")).toBe(true);
    expect(p.band.endsWith("Z")).toBe(true);
    expect(p.area).toContain("L100.0,50.0");
    expect(timeX(500, s, 100)).toBe(50);
    expect(nearest(s, 49, 100)).toEqual([500, 10, 40]);
    expect(seriesPaths([], 100, 50, 50).line).toBe("");
  });

  it("keeps one freeze from flattening the axis", () => {
    const s: [number, number, number][] = Array.from({ length: 100 }, (_, i) => [i, 16, i === 50 ? 900 : 18]);
    expect(frametimeAxis(s)).toBe(25);
  });

  it("only offers finished sessions with frames for compare", () => {
    const base = { label: "", method: "presentmon", target: {}, started_ms: 0, stutters: 0, freezes: 0 };
    const list: SessionSummary[] = [
      { ...base, session_id: "a", status: "done", frames: 10 },
      { ...base, session_id: "b", status: "failed", frames: 0 },
      { ...base, session_id: "c", status: "done", frames: 0 },
    ];
    expect(comparable(list).map((s) => s.session_id)).toEqual(["a"]);
  });
});
