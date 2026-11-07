import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

type Entry = {
  id: string;
  kind: "recording" | "export" | "snapshot";
  path: string;
  created_at: number;
  duration: number | null;
  size_bytes: number;
  source: string | null;
  exists: boolean;
};

const DIR = "/preview/Movies";
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const stamp = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}-${String(d.getMinutes()).padStart(2, "0")}-${String(d.getSeconds()).padStart(2, "0")}`;

const SOURCES = {
  platform: "macos",
  ffmpeg: true,
  screens: [
    { id: "2", name: "Capture screen 0", kind: "screen" },
    { id: "3", name: "Capture screen 1", kind: "screen" },
  ],
  audio: [
    { id: "0", name: "Microsoft Teams Audio", kind: "audio" },
    { id: "1", name: "MacBook Air Microphone", kind: "audio" },
    { id: "2", name: "CORSAIR VIRTUOSO XT Wireless Gaming Receiver", kind: "audio" },
  ],
  error: null,
};

let history: Entry[] = [
  { id: "h1", kind: "recording", path: `${DIR}/OmniGet Capture 2026-11-04 16-20-11.mp4`, created_at: Date.now() - 86_400_000, duration: 312.4, size_bytes: 184_000_000, source: null, exists: true },
  { id: "h2", kind: "export", path: `${DIR}/OmniGet Capture 2026-11-04 16-20-11 (cut).mp4`, created_at: Date.now() - 80_000_000, duration: 95.2, size_bytes: 56_200_000, source: `${DIR}/OmniGet Capture 2026-11-04 16-20-11.mp4`, exists: true },
  { id: "h3", kind: "snapshot", path: `${DIR}/demo talk [01m12s300].png`, created_at: Date.now() - 70_000_000, duration: null, size_bytes: 2_400_000, source: `${DIR}/demo talk.mp4`, exists: true },
  { id: "h4", kind: "recording", path: `${DIR}/Old/OmniGet Capture 2026-10-30 09-02-45.mov`, created_at: Date.now() - 6 * 86_400_000, duration: 61, size_bytes: 40_000_000, source: null, exists: false },
];

let active: { id: string; path: string; started_at: number } | null = null;
const cancelled = new Set<string>();

function add(entry: Omit<Entry, "id" | "created_at" | "exists">) {
  history = [{ ...entry, id: `h${Date.now()}${Math.random().toString(36).slice(2, 6)}`, created_at: Date.now(), exists: true }, ...history];
}

function probe(path: string) {
  const name = path.split("/").pop() ?? path;
  if (name.startsWith("broken")) throw new Error("CAPTURE_PROBE_FAILED: Invalid data found when processing input");
  const duration = name.includes("Capture") ? 312.4 : 95.43;
  const keyframes: number[] = [];
  for (let t = 0; t < duration; t += 2.002) keyframes.push(Math.round(t * 1000) / 1000);
  return {
    path,
    name,
    size_bytes: Math.round(duration * 590_000),
    duration,
    format_name: "mov,mp4,m4a,3gp,3g2,mj2",
    video: { codec: "h264", width: 2940, height: 1912, fps: 30 },
    audio: [{ index: 1, codec: "aac", channels: 2 }],
    keyframes,
  };
}

async function runExport(args: Args) {
  const jobId = String(args.jobId);
  const req = args.request as { path: string; segments: { start: number; end: number }[]; mode: string; out_dir: string | null };
  const segs = req.segments ?? [];
  if (segs.length === 0) throw new Error("CAPTURE_NO_SEGMENTS");
  const merge = req.mode === "merge" && segs.length > 1;
  const steps = segs.length + (merge ? 1 : 0);
  for (let p = 0; p <= 95; p += 5) {
    if (cancelled.has(jobId)) {
      cancelled.delete(jobId);
      await emit("capture-export-progress", { job_id: jobId, percent: 0, step: steps, steps, state: "cancelled", error: "CAPTURE_CANCELLED" });
      throw new Error("CAPTURE_CANCELLED");
    }
    await emit("capture-export-progress", { job_id: jobId, percent: p, step: Math.min(steps, 1 + Math.floor((p / 100) * steps)), steps, state: "running" });
    await sleep(120);
  }
  const dir = req.out_dir ?? req.path.slice(0, req.path.lastIndexOf("/"));
  const name = req.path.split("/").pop() ?? "video.mp4";
  const stem = name.replace(/\.[^.]+$/, "");
  const ext = name.split(".").pop() ?? "mp4";
  const fmt = (t: number) => `${String(Math.floor(t / 60)).padStart(2, "0")}m${String(Math.floor(t % 60)).padStart(2, "0")}s${String(Math.round((t % 1) * 1000)).padStart(3, "0")}`;
  const outputs = req.mode === "merge" ? [`${dir}/${stem} (cut).${ext}`] : segs.map((s) => `${dir}/${stem} [${fmt(s.start)} - ${fmt(s.end)}].${ext}`);
  for (const out of outputs) add({ kind: "export", path: out, duration: 10, size_bytes: 8_400_000, source: req.path });
  await emit("capture-export-progress", { job_id: jobId, percent: 100, step: steps, steps, state: "done" });
  return { outputs, size_bytes: outputs.length * 8_400_000 };
}

export const captureFixtures: Record<string, (args: Args) => unknown> = {
  capture_sources: () => SOURCES,
  capture_default_dir: () => "/preview/Downloads",
  capture_status: () => active,
  capture_start: async ({ options }) => {
    const o = options as { screen: string; container: string; out_dir: string | null };
    await sleep(500);
    if (o.screen === "3") {
      throw new Error("CAPTURE_PERMISSION_SCREEN: [in#0 @ 0x9a] Error opening input: Input/output error Error opening input file 3:1.");
    }
    const path = `${o.out_dir ?? "/preview/Downloads"}/OmniGet Capture ${stamp(new Date())}.${o.container}`;
    active = { id: `r${Date.now()}`, path, started_at: Date.now() };
    return active;
  },
  capture_stop: async () => {
    if (!active) throw new Error("CAPTURE_NOT_RECORDING");
    await sleep(900);
    const duration = (Date.now() - active.started_at) / 1000;
    const outcome = { path: active.path, ok: true, duration, size_bytes: Math.round(duration * 1_100_000), forced: false, stopped_by_user: true, error: null };
    add({ kind: "recording", path: active.path, duration, size_bytes: outcome.size_bytes, source: null });
    active = null;
    return outcome;
  },
  capture_probe: async ({ path }) => {
    await sleep(400);
    return probe(String(path));
  },
  capture_export: runExport,
  capture_cancel: ({ jobId }) => {
    cancelled.add(String(jobId));
    return null;
  },
  capture_snapshot: async ({ path, at }) => {
    await sleep(300);
    const p = String(path);
    const stem = (p.split("/").pop() ?? "video").replace(/\.[^.]+$/, "");
    const t = Number(at) || 0;
    const out = `${p.slice(0, p.lastIndexOf("/"))}/${stem} [${String(Math.floor(t / 60)).padStart(2, "0")}m${String(Math.floor(t % 60)).padStart(2, "0")}s000].png`;
    add({ kind: "snapshot", path: out, duration: null, size_bytes: 2_100_000, source: p });
    return out;
  },
  capture_history: () => history,
  capture_history_remove: ({ id }) => {
    history = history.filter((e) => e.id !== id);
    return history;
  },
  capture_open_privacy: () => null,
};

export function captureDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/capture")) return undefined;
  if (options?.directory) return "/preview/Movies/Exports";
  return `${DIR}/demo talk.mp4`;
}
