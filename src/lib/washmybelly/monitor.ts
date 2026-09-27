/// Types and pure helpers of the Overview module (health score, live
/// monitor, processes). The Rust side is `core/washmybelly/monitor/`.

export type Severity = "ok" | "warn" | "bad";

export type Factor = {
  id: "disk_space" | "temperature" | "battery" | "memory" | "junk" | "login_items" | "privacy";
  penalty: number;
  severity: Severity;
  value: number;
  detail: string;
};

export type Gain = {
  id: "junk" | "free_space" | "login_items" | "battery";
  /// Module that acts on it (`clean`, `disk`, `startup`, `power`).
  module: string;
  bytes?: number;
  count?: number;
  percent?: number;
  detail: string;
};

export type Battery = {
  present: boolean;
  cycles?: number;
  health_percent?: number;
  design_capacity?: number;
  full_charge_capacity?: number;
  capacity_unit?: "mAh" | "mWh";
  charge_percent?: number;
  temperature_c?: number;
  state: "charging" | "discharging" | "full" | "not_charging" | "unknown";
  on_ac?: boolean;
  power_w?: number;
  time_remaining_min?: number;
  source: string;
};

export type SystemStatus = {
  score: number;
  grade: "excellent" | "good" | "fair" | "poor";
  factors: Factor[];
  gains: Gain[];
  hooks: { junk: boolean; login_items: boolean; privacy: boolean };
  cpu: { usage_percent: number; cores: number; brand?: string | null; frequency_mhz?: number | null; load_average?: number[] | null };
  memory: { total: number; used: number; available: number; swap_total: number; swap_used: number; pressure_percent?: number | null };
  disk?: { mount: string; name: string; total: number; free: number; free_percent: number } | null;
  temperature: { cpu_c?: number | null; gpu_c?: number | null; reason?: string };
  battery?: Battery | null;
  uptime_secs: number;
};

export type LiveTick = {
  t: number;
  cpu: { total: number; cores: number[] };
  memory: { total: number; used: number; available: number; swap_total: number; swap_used: number; pressure_percent?: number | null };
  net: { rx_per_s: number; tx_per_s: number };
  disk: { read_per_s: number; write_per_s: number };
  gpu_percent: number | null;
  temp_c: number | null;
  gpu_temp_c?: number | null;
  iowait_percent?: number | null;
};

export type ProcessRow = {
  pid: number;
  parent_pid?: number | null;
  name: string;
  user?: string | null;
  cpu_percent: number;
  memory_bytes: number;
  disk_read_per_s: number;
  disk_write_per_s: number;
  command: string;
  started: number;
  status: string;
  mine: boolean;
  can_end: boolean;
};

export type ProcessList = { processes: ProcessRow[]; total: number; sort: string; cpu_count: number };

export type SortKey = "name" | "cpu" | "memory" | "disk" | "pid" | "user";
export type SortDir = "asc" | "desc";

/// Samples kept per chart: one a second, two minutes.
export const HISTORY = 120;

/// Append a sample, keeping the last `max` (a new array, for Svelte).
export function pushSample(buf: number[], v: number | null | undefined, max = HISTORY): number[] {
  if (v === null || v === undefined || !Number.isFinite(v)) return buf;
  const next = buf.length >= max ? buf.slice(buf.length - max + 1) : buf.slice();
  next.push(v);
  return next;
}

/// Everything the live charts keep between ticks.
export type Series = { cpu: number[]; memory: number[]; gpu: number[]; temp: number[]; rx: number[]; tx: number[]; read: number[]; write: number[] };

export function emptySeries(): Series {
  return { cpu: [], memory: [], gpu: [], temp: [], rx: [], tx: [], read: [], write: [] };
}

export function memoryPercent(m: { total: number; used: number }): number {
  return m.total > 0 ? Math.round((m.used / m.total) * 1000) / 10 : 0;
}

export function addTick(s: Series, t: LiveTick): Series {
  return {
    cpu: pushSample(s.cpu, t.cpu.total),
    memory: pushSample(s.memory, memoryPercent(t.memory)),
    gpu: pushSample(s.gpu, t.gpu_percent),
    temp: pushSample(s.temp, t.temp_c),
    rx: pushSample(s.rx, t.net.rx_per_s),
    tx: pushSample(s.tx, t.net.tx_per_s),
    read: pushSample(s.read, t.disk.read_per_s),
    write: pushSample(s.write, t.disk.write_per_s),
  };
}

/// A chart ceiling that does not jump on every tick: the next "nice" step
/// above the largest sample (1, 2, 5 × 10^n), at least `floor`.
export function niceMax(values: number[], floor = 1): number {
  const m = Math.max(floor, ...values.filter((v) => Number.isFinite(v)));
  const p = 10 ** Math.floor(Math.log10(m));
  for (const k of [1, 2, 5, 10]) if (k * p >= m) return k * p;
  return 10 * p;
}

/// SVG path of a line (and of the area under it) for `values` in a box.
export function linePath(values: number[], max: number, width: number, height: number, slots = HISTORY): { line: string; area: string } {
  if (values.length === 0 || max <= 0) return { line: "", area: "" };
  const step = width / Math.max(1, slots - 1);
  const x0 = width - (values.length - 1) * step;
  const pts = values.map((v, i) => {
    const x = x0 + i * step;
    const y = height - (Math.max(0, Math.min(v, max)) / max) * height;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const line = `M${pts.join(" L")}`;
  const area = `${line} L${width.toFixed(1)},${height} L${x0.toFixed(1)},${height} Z`;
  return { line, area };
}

const RATE_UNITS = ["B/s", "KB/s", "MB/s", "GB/s"];

/// Decimal rate, like `formatBytes`.
export function formatRate(n: number | null | undefined): string {
  if (!n || n < 0) return "0 B/s";
  let v = n;
  let i = 0;
  while (v >= 1000 && i < RATE_UNITS.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${RATE_UNITS[i]}`;
}

export function formatPercent(n: number | null | undefined): string {
  return n === null || n === undefined || !Number.isFinite(n) ? "—" : `${Math.round(n)}%`;
}

export function formatTemp(n: number | null | undefined): string {
  return n === null || n === undefined || !Number.isFinite(n) ? "—" : `${Math.round(n)} °C`;
}

/// CPU of one process as a share of the whole machine (sysinfo gives 100 per core).
export function machineCpu(cpuPercent: number, cpuCount: number): number {
  return cpuCount > 0 ? cpuPercent / cpuCount : cpuPercent;
}

export function filterProcesses(rows: ProcessRow[], query: string): ProcessRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((r) => r.name.toLowerCase().includes(q) || r.command.toLowerCase().includes(q) || String(r.pid) === q || (r.user ?? "").toLowerCase() === q);
}

function keyOf(r: ProcessRow, k: SortKey): number | string {
  switch (k) {
    case "name":
      return r.name.toLowerCase();
    case "user":
      return (r.user ?? "").toLowerCase();
    case "pid":
      return r.pid;
    case "memory":
      return r.memory_bytes;
    case "disk":
      return r.disk_read_per_s + r.disk_write_per_s;
    default:
      return r.cpu_percent;
  }
}

/// Stable sort; ties keep the pid order so rows do not dance between ticks.
export function sortProcesses(rows: ProcessRow[], key: SortKey, dir: SortDir): ProcessRow[] {
  const s = dir === "asc" ? 1 : -1;
  return rows
    .map((r, i) => ({ r, i }))
    .sort((a, b) => {
      const x = keyOf(a.r, key);
      const y = keyOf(b.r, key);
      const d = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
      return d !== 0 ? d * s : a.r.pid - b.r.pid;
    })
    .map((x) => x.r);
}

/// Next sort state when a column header is clicked: a new column starts at
/// its natural order (text ascending, numbers descending), the same column flips.
export function toggleSort(cur: { key: SortKey; dir: SortDir }, key: SortKey): { key: SortKey; dir: SortDir } {
  if (cur.key !== key) return { key, dir: key === "name" || key === "user" || key === "pid" ? "asc" : "desc" };
  return { key, dir: cur.dir === "asc" ? "desc" : "asc" };
}

/// Colour of a score, in step with `ui/HealthRing`.
export function scoreTone(score: number): "good" | "fair" | "poor" {
  return score >= 80 ? "good" : score >= 60 ? "fair" : "poor";
}

/// Error codes the core puts at the start of a refusal (`REFUSED: ...`).
export function errorCode(e: unknown): "REFUSED" | "NOT_FOUND" | "CHANGED" | "NEEDS_ADMIN" | "OTHER" {
  const s = String(e);
  for (const c of ["REFUSED", "NOT_FOUND", "CHANGED", "NEEDS_ADMIN"] as const) if (s.includes(`${c}:`)) return c;
  return "OTHER";
}

/// The reading behind a factor, formatted for its row.
export function factorValue(f: Factor, format: (n: number) => string): string {
  switch (f.id) {
    case "disk_space":
      return `${Math.round(f.value)}%`;
    case "temperature":
      return formatTemp(f.value);
    case "battery":
    case "memory":
      return `${Math.round(f.value)}%`;
    case "junk":
      return format(f.value);
    default:
      return String(Math.round(f.value));
  }
}
