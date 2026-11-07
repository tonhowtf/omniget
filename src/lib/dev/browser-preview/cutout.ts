import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;
type Edge = { feather: number; shift: number };
type Background = { kind: "none" } | { kind: "color"; color: string } | { kind: "image"; path: string };

const downloaded = new Set(["u2netp"]);
const cancelled = new Set<string>();
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const MODELS = [
  { id: "u2netp", name: "U²-Net light", bytes: 4_574_861 },
  { id: "silueta", name: "Silueta", bytes: 44_173_029 },
  { id: "isnet-general-use", name: "IS-Net", bytes: 178_648_008 },
];

function status() {
  return {
    runtime_installed: true,
    runtime_downloadable: true,
    models: MODELS.map((m) => ({ ...m, license: "Apache-2.0", source: "https://github.com/xuebinqin/U-2-Net", downloaded: downloaded.has(m.id) })),
    refine: { id: "sam21", name: "SAM 2.1 tiny", bytes: 130_129_714, license: "Apache-2.0", source: "https://huggingface.co/cstr/sam2.1-hiera-tiny-ONNX", downloaded: downloaded.has("sam21") },
  };
}

function isVideo(path: string) {
  return /\.(mp4|m4v|mkv|webm|mov|avi)$/i.test(path);
}

function inspectOne(path: string) {
  const name = path.split("/").pop() ?? path;
  if (name.startsWith("empty")) return { path, name, info: null, error: "The file is empty (0 bytes)." };
  const video = isVideo(path);
  return {
    path,
    name,
    info: { path, name, kind: video ? "video" : "image", width: video ? 1920 : 1200, height: video ? 1080 : 900, duration_ms: video ? 94_000 : 0 },
    error: null,
  };
}

function scene(ctx: CanvasRenderingContext2D, w: number, h: number, frameMs: number | null) {
  const sky = ctx.createLinearGradient(0, 0, 0, h);
  sky.addColorStop(0, "#9cc9ff");
  sky.addColorStop(1, "#3f7fd8");
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = "#2f6b3a";
  ctx.fillRect(0, h * 0.78, w, h * 0.22);
  if (frameMs !== null) {
    ctx.fillStyle = "rgba(255,255,255,0.8)";
    ctx.font = `${Math.round(h / 18)}px system-ui`;
    ctx.fillText(`${(frameMs / 1000).toFixed(1)} s`, w * 0.04, h * 0.1);
  }
}

function subject(ctx: CanvasRenderingContext2D, w: number, h: number, grow: number) {
  ctx.fillStyle = "#e0453a";
  ctx.beginPath();
  ctx.arc(w * 0.5, h * 0.55, Math.max(4, h * 0.28 + grow), 0, Math.PI * 2);
  ctx.fill();
}

function preview(args: Args) {
  const frameMs = (args.frameMs as number | null) ?? null;
  const edge = (args.edge as Edge) ?? { feather: 0, shift: 0 };
  const bg = (args.background as Background) ?? { kind: "none" };
  const w = 640;
  const h = isVideo(String(args.path)) ? 360 : 480;
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;
  scene(ctx, w, h, frameMs);
  subject(ctx, w, h, 0);
  const before = canvas.toDataURL("image/jpeg", 0.85);

  ctx.clearRect(0, 0, w, h);
  if (bg.kind === "color") {
    ctx.fillStyle = bg.color;
    ctx.fillRect(0, 0, w, h);
  } else if (bg.kind === "image") {
    const g = ctx.createLinearGradient(0, 0, w, h);
    g.addColorStop(0, "#f7d9a8");
    g.addColorStop(1, "#c98b5a");
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
  }
  const scale = Math.max(w, h) / 1000;
  ctx.filter = edge.feather > 0 ? `blur(${edge.feather * scale}px)` : "none";
  subject(ctx, w, h, edge.shift * scale);
  ctx.filter = "none";
  return { before, after: canvas.toDataURL("image/png"), width: w, height: h };
}

async function install(args: Args) {
  const model = String(args.model);
  const total = model === "sam21" ? 130_129_714 : (MODELS.find((m) => m.id === model)?.bytes ?? 1);
  for (const step of [0, 0.2, 0.45, 0.7, 0.9, 1]) {
    await emit("cutout-install-progress", { model, id: `onnx-model:${model}`, stage: "download", done: Math.round(total * step), total });
    await sleep(220);
  }
  downloaded.add(model);
  return status();
}

async function run(args: Args) {
  const batchId = String(args.batchId);
  const items = (args.items as { path: string; frame_ms: number | null }[]) ?? [];
  const options = (args.options as { format: string; out_dir: string | null }) ?? { format: "png", out_dir: null };
  const outcomes = [];
  for (let index = 0; index < items.length; index++) {
    const { path, frame_ms } = items[index];
    if (cancelled.has(batchId)) {
      const outcome = { input: path, frame_ms, output: null, size_bytes: 0, ok: false, error: "Cancelled." };
      await emit("cutout-progress", { batch_id: batchId, index, state: "failed", outcome });
      outcomes.push(outcome);
      continue;
    }
    await emit("cutout-progress", { batch_id: batchId, index, state: "running" });
    await sleep(isVideo(path) ? 900 : 500);
    const dir = options.out_dir ?? path.slice(0, path.lastIndexOf("/"));
    const stem = (path.split("/").pop() ?? "picture").replace(/\.[^.]+$/, "");
    const moment = frame_ms === null ? "" : `-${String(Math.floor(frame_ms / 60000)).padStart(2, "0")}m${String(Math.floor(frame_ms / 1000) % 60).padStart(2, "0")}s`;
    const outcome = { input: path, frame_ms, output: `${dir}/${stem}${moment}-cutout.${options.format}`, size_bytes: 840_000, ok: true, error: null };
    await emit("cutout-progress", { batch_id: batchId, index, state: "done", outcome });
    outcomes.push(outcome);
  }
  cancelled.delete(batchId);
  return outcomes;
}

export const cutoutFixtures: Record<string, (args: Args) => unknown> = {
  cutout_status: status,
  cutout_install: install,
  cutout_inspect: ({ paths }) => ((paths as string[]) ?? []).map(inspectOne),
  cutout_preview: preview,
  cutout_run: run,
  cutout_cancel: ({ batchId }) => {
    cancelled.add(String(batchId));
    return null;
  },
  cutout_release: () => null,
};

export function cutoutDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/cutout")) return undefined;
  if (options?.directory) return "/preview/Cutouts";
  if (options?.multiple === false) return "/preview/Pictures/beach.jpg";
  return ["/preview/Pictures/portrait.jpg", "/preview/Pictures/produto – foto de catálogo.png", "/preview/Movies/trip.mp4", "/preview/Pictures/empty.png"];
}
