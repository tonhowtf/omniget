export type PerfCaps = {
  available: boolean;
  method: "presentmon" | "mangohud" | "metal_hud";
  missing: string[];
  frames: boolean | "launched_only";
  reason?: string | null;
  os: "windows" | "linux" | "macos";
  telemetry: boolean;
  active?: string | null;
  windows?: { performance_log_users: boolean; is_admin: boolean; setup_action?: string | null; setup_needs_sign_out?: boolean };
  presentmon?: { installed: boolean; version: string; download_on_first_use: boolean };
  linux?: { mangohud: boolean; flatpak: boolean };
  macos?: { hud_log_readable: boolean; frames_when_started_by_omniget: boolean };
};

export type LaunchCommand = { kind: string; command: string };

export type SessionSummary = {
  session_id: string;
  label: string;
  method: string;
  status: "recording" | "waiting_launch" | "done" | "failed";
  target: { name?: string | null; pid?: number | null };
  started_ms: number;
  ended_ms?: number | null;
  frames: number;
  fps_avg?: number | null;
  low_1?: number | null;
  low_01?: number | null;
  stutters: number;
  freezes: number;
  reason?: string | null;
};

export type Stats = {
  frames: number;
  duration_s: number;
  fps_avg?: number | null;
  frametime_avg_ms?: number | null;
  fps_min?: number | null;
  fps_max?: number | null;
  fps_p99?: number | null;
  fps_p95?: number | null;
  fps_median?: number | null;
  fps_p5?: number | null;
  fps_p1?: number | null;
  fps_p01?: number | null;
  frametime_median_ms?: number | null;
  frametime_p99_ms?: number | null;
  frametime_p99_9_ms?: number | null;
  low_1?: number | null;
  low_01?: number | null;
  low_1_integral?: number | null;
  low_01_integral?: number | null;
  stutters: number;
  stutter_time_pct?: number | null;
  low_fps_time_pct?: number | null;
  freezes: number;
  freeze_time_ms: number;
  variance: number[];
};

type FrameEvent = { t_ms: number; frametime_ms: number };

export type Summary = Stats & {
  session_id: string;
  label: string;
  method: string;
  source: "present" | "display";
  stutter_events: FrameEvent[];
  freeze_events: FrameEvent[];
  display_stats?: Stats | null;
  sensors: { samples: number; cpu_load_avg?: number | null; gpu_load_avg?: number | null; cpu_temp_max?: number | null; gpu_temp_max?: number | null; power_avg_w?: number | null };
  correlation: { findings: string[]; stutters_checked: number; stutters_hot: number; stutters_gpu_full: number; stutters_cpu_full: number; gpu_full_pct?: number | null };
  warnings: string[];
  reason?: string | null;
  formulas: string;
};

export type Histogram = { start_ms: number; width_ms: number; counts: number[]; overflow: number };

type SensorRow = { t_ms: number; cpu_load?: number | null; cpu_temp?: number | null; gpu_load?: number | null; gpu_temp?: number | null };

export type Chart = {
  session_id: string;
  source: "present" | "display";
  series: [number, number, number][];
  histogram: Histogram;
  sensors: SensorRow[];
  stutter_events: FrameEvent[];
  freeze_events: FrameEvent[];
  duration_ms: number;
};

export type Delta = {
  metric: string;
  before?: number | null;
  after?: number | null;
  delta?: number | null;
  delta_pct?: number | null;
  noise_pct?: number | null;
  verdict: "better" | "worse" | "same" | "unknown";
};

export type Compare = {
  before: SessionSummary & { fps_avg?: number | null };
  after: SessionSummary & { fps_avg?: number | null };
  delta: { verdict: "better" | "worse" | "same" | "mixed"; metrics: Delta[]; stutters: number; freezes: number };
  warnings: string[];
};

export type LiveStatus =
  | { active: false }
  | {
      active: true;
      session_id: string;
      status: string | null;
      method: string | null;
      label: string | null;
      launch: LaunchCommand[];
      elapsed_s: number;
      max_seconds: number | null;
      frames: number;
      fps_now: number | null;
      recent: number[];
    };

export function formatFps(v: number | null | undefined): string {
  if (v == null || !Number.isFinite(v)) return "—";
  return v >= 100 ? String(Math.round(v)) : v.toFixed(1);
}

export function formatMs(v: number | null | undefined, digits = 1): string {
  if (v == null || !Number.isFinite(v)) return "—";
  return `${v.toFixed(digits)} ms`;
}

export function formatClock(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

export function formatDelta(d: Delta): string {
  const v = d.delta_pct;
  if (v == null || !Number.isFinite(v)) return "—";
  const unit = d.metric.endsWith("_pct") ? " pt" : "%";
  if (Math.abs(v) < 0.05) return `0${unit}`;
  const sign = v > 0 ? "+" : "−";
  return `${sign}${Math.abs(v).toFixed(1)}${unit}`;
}

export function smoothness(avg: number | null | undefined, low: number | null | undefined): "good" | "warn" | "bad" | "default" {
  if (!avg || !low) return "default";
  const r = low / avg;
  if (r >= 0.7) return "good";
  if (r >= 0.5) return "warn";
  return "bad";
}

export function niceMax(v: number): number {
  if (!(v > 0) || !Number.isFinite(v)) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
  return 10 * p;
}

export type Bar = { x: number; y: number; w: number; h: number; from: number; to: number; count: number; overflow?: boolean };

export function histogramBars(h: Histogram, width: number, height: number, gap = 1): Bar[] {
  const n = h.counts.length + (h.overflow > 0 ? 1 : 0);
  if (n === 0 || width <= 0 || height <= 0) return [];
  const max = Math.max(1, ...h.counts, h.overflow);
  const slot = width / n;
  const bars: Bar[] = h.counts.map((c, i) => {
    const bh = c > 0 ? Math.max(1, (c / max) * height) : 0;
    return { x: i * slot, y: height - bh, w: Math.max(0.5, slot - gap), h: bh, from: h.start_ms + i * h.width_ms, to: h.start_ms + (i + 1) * h.width_ms, count: c };
  });
  if (h.overflow > 0) {
    const bh = Math.max(1, (h.overflow / max) * height);
    const from = h.start_ms + h.counts.length * h.width_ms;
    bars.push({ x: (n - 1) * slot, y: height - bh, w: Math.max(0.5, slot - gap), h: bh, from, to: Infinity, count: h.overflow, overflow: true });
  }
  return bars;
}

export function seriesPaths(series: [number, number, number][], width: number, height: number, yMax: number): { band: string; line: string; area: string } {
  if (series.length === 0 || width <= 0 || height <= 0) return { band: "", line: "", area: "" };
  const t0 = series[0][0];
  const t1 = series[series.length - 1][0];
  const span = t1 - t0 || 1;
  const x = (t: number) => ((t - t0) / span) * width;
  const y = (v: number) => height - (Math.min(v, yMax) / yMax) * height;
  const f = (n: number) => n.toFixed(1);
  const top = series.map(([t, , mx]) => `${f(x(t))},${f(y(mx))}`);
  const bottom = series.map(([t, mn]) => `${f(x(t))},${f(y(mn))}`).reverse();
  const line = `M${top.join(" L")}`;
  return {
    band: `M${top.join(" L")} L${bottom.join(" L")} Z`,
    line,
    area: `${line} L${f(x(t1))},${f(height)} L${f(x(t0))},${f(height)} Z`,
  };
}

export function timeX(t: number, series: [number, number, number][], width: number): number {
  if (series.length === 0) return 0;
  const t0 = series[0][0];
  const span = series[series.length - 1][0] - t0 || 1;
  return Math.min(width, Math.max(0, ((t - t0) / span) * width));
}

export function nearest(series: [number, number, number][], x: number, width: number): [number, number, number] | null {
  if (series.length === 0 || width <= 0) return null;
  const i = Math.round((Math.min(Math.max(x, 0), width) / width) * (series.length - 1));
  return series[i] ?? null;
}

export function frametimeAxis(series: [number, number, number][]): number {
  if (series.length === 0) return 50;
  const maxes = series.map((s) => s[2]).sort((a, b) => a - b);
  const p = maxes[Math.min(maxes.length - 1, Math.floor(maxes.length * 0.98))];
  return niceMax(Math.max(p * 1.15, 8));
}

export const METRIC_KEYS: Record<string, string> = {
  fps_avg: "metric_fps_avg",
  fps_median: "metric_fps_median",
  low_1: "metric_low_1",
  low_01: "metric_low_01",
  low_1_integral: "metric_low_1_integral",
  fps_p1: "metric_fps_p1",
  frametime_avg_ms: "metric_frametime_avg",
  frametime_p99_ms: "metric_frametime_p99",
  stutter_time_pct: "metric_stutter_time",
  low_fps_time_pct: "metric_low_fps_time",
};

export function formatMetric(metric: string, v: number | null | undefined): string {
  if (v == null || !Number.isFinite(v)) return "—";
  if (metric.endsWith("_ms")) return formatMs(v);
  if (metric.endsWith("_pct")) return `${v.toFixed(1)}%`;
  return formatFps(v);
}

export function comparable(list: SessionSummary[]): SessionSummary[] {
  return list.filter((s) => s.status === "done" && s.frames > 0);
}
