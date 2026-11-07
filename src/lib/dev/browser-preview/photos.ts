import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;
type Edits = { exposure: number; contrast: number; saturation: number; temperature: number; rotate: number; crop: { x: number; y: number; w: number; h: number } | null };

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const at = (y: number, m: number, d: number, h = 12) => Date.UTC(y, m - 1, d, h) / 1000;

let folders = ["/preview/Downloads/OmniGet", "/preview/Pictures"];
let chosen = false;
const edits = new Map<string, Edits>();
const cancelled = new Set<string>();
const thumbs = new Map<string, string>();

const SEEDS: [string, number, number, number, number | null, string][] = [
  ["/preview/Pictures/Trip/IMG_0421.HEIC", 4032, 3024, 2_400_000, at(2026, 10, 12, 9), "#3f7fd8"],
  ["/preview/Pictures/Trip/IMG_0422.HEIC", 4032, 3024, 2_380_000, at(2026, 10, 12, 9), "#3f80d9"],
  ["/preview/Pictures/Trip/IMG_0423.jpg", 3024, 4032, 3_100_000, at(2026, 10, 12, 17), "#e0753a"],
  ["/preview/Pictures/Trip/DSC_1180.NEF", 6000, 4000, 24_800_000, at(2026, 10, 13, 8), "#2f6b3a"],
  ["/preview/Pictures/Family/birthday.jpg", 2048, 1536, 1_200_000, at(2026, 8, 3, 20), "#c84a7b"],
  ["/preview/Pictures/Family/birthday (1).jpg", 2048, 1536, 1_200_000, at(2026, 8, 3, 20), "#c84a7b"],
  ["/preview/Pictures/Family/cake.png", 1600, 1200, 3_900_000, at(2026, 8, 3, 21), "#f2b33d"],
  ["/preview/Downloads/OmniGet/wallpaper.webp", 3840, 2160, 980_000, null, "#7b5cff"],
  ["/preview/Downloads/OmniGet/cover.jxl", 1400, 1400, 420_000, null, "#14907f"],
  ["/preview/Downloads/OmniGet/mockup.psd", 2400, 1800, 18_000_000, null, "#555"],
];

function art(color: string, w: number, h: number, e: Edits | null): string {
  const canvas = document.createElement("canvas");
  const turned = e ? e.rotate % 180 === 90 : false;
  canvas.width = turned ? h : w;
  canvas.height = turned ? w : h;
  const ctx = canvas.getContext("2d")!;
  ctx.save();
  ctx.translate(canvas.width / 2, canvas.height / 2);
  ctx.rotate(((e?.rotate ?? 0) * Math.PI) / 180);
  ctx.translate(-w / 2, -h / 2);
  const sky = ctx.createLinearGradient(0, 0, 0, h);
  sky.addColorStop(0, "#fff");
  sky.addColorStop(1, color);
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = "rgba(0,0,0,0.35)";
  ctx.beginPath();
  ctx.moveTo(0, h);
  ctx.lineTo(w * 0.35, h * 0.45);
  ctx.lineTo(w * 0.6, h * 0.75);
  ctx.lineTo(w * 0.8, h * 0.55);
  ctx.lineTo(w, h);
  ctx.fill();
  ctx.fillStyle = "#ffd76a";
  ctx.beginPath();
  ctx.arc(w * 0.75, h * 0.25, Math.min(w, h) * 0.08, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
  if (e) {
    const b = 1 + e.exposure / 100;
    const c = 1 + e.contrast / 150;
    const s = 1 + e.saturation / 100;
    const out = document.createElement("canvas");
    const crop = e.crop ?? { x: 0, y: 0, w: 1, h: 1 };
    out.width = Math.max(1, Math.round(canvas.width * crop.w));
    out.height = Math.max(1, Math.round(canvas.height * crop.h));
    const o = out.getContext("2d")!;
    o.filter = `brightness(${b}) contrast(${c}) saturate(${Math.max(0, s)}) sepia(${Math.max(0, e.temperature) / 300})`;
    o.drawImage(canvas, -crop.x * canvas.width, -crop.y * canvas.height);
    return out.toDataURL("image/jpeg", 0.85);
  }
  return canvas.toDataURL("image/jpeg", 0.8);
}

function photo([path, width, height, size, taken, color]: (typeof SEEDS)[number]) {
  const name = path.split("/").pop()!;
  if (!thumbs.has(path)) {
    const s = 360 / Math.max(width, height);
    thumbs.set(path, art(color, Math.round(width * s), Math.round(height * s), null));
  }
  return {
    path,
    name,
    folder: path.slice(0, path.lastIndexOf("/")),
    format: name.split(".").pop()!.toLowerCase(),
    size,
    modified_at: taken ?? at(2026, 11, 2, 15),
    taken_at: taken,
    width,
    height,
    thumb: thumbs.get(path)!,
    error: null,
    edited: edits.has(path),
  };
}

let present = new Set(SEEDS.map((s) => s[0]));

function list() {
  return SEEDS.filter((s) => present.has(s[0])).map(photo);
}

function status(stage: string, extra: Partial<{ found: number; total: number; done: number; failed: number }> = {}) {
  return { stage, found: 0, total: 0, done: 0, failed: 0, error: null, ...extra };
}

let last = status("idle");

async function index() {
  last = status("scanning");
  void (async () => {
    await sleep(400);
    last = status("indexing", { found: present.size, total: 4 });
    await emit("photos-index-progress", last);
    for (let i = 1; i <= 4; i++) {
      await sleep(350);
      last = status("indexing", { found: present.size, total: 4, done: i });
      await emit("photos-index-progress", last);
    }
    last = status("done", { found: present.size, total: 4, done: 4 });
    await emit("photos-index-progress", last);
  })();
  return last;
}

function findSeed(path: string) {
  return SEEDS.find((s) => s[0] === path)!;
}

function preview(args: Args) {
  const seed = findSeed(String(args.path));
  const e = args.edits as Edits;
  const s = 1600 / Math.max(seed[1], seed[2]);
  const w = Math.round(seed[1] * s);
  const h = Math.round(seed[2] * s);
  const src = art(seed[5], w, h, e);
  const turned = e.rotate % 180 === 90;
  const crop = e.crop ?? { x: 0, y: 0, w: 1, h: 1 };
  return {
    src,
    width: Math.round((turned ? h : w) * crop.w),
    height: Math.round((turned ? w : h) * crop.h),
    full_width: turned ? seed[2] : seed[1],
    full_height: turned ? seed[1] : seed[2],
  };
}

function report(paths: string[], moved: boolean) {
  for (const p of paths) present.delete(p);
  return { done: paths, moved_to: moved ? paths.map((p) => `/preview/Moved/${p.split("/").pop()}`) : [], failed: [] };
}

async function runExport(args: Args) {
  const batchId = String(args.batchId);
  const paths = (args.paths as string[]) ?? [];
  const options = args.options as { format: string; out_dir: string | null };
  const ext = options.format === "jpeg" ? "jpg" : options.format;
  const outcomes = [];
  for (let index = 0; index < paths.length; index++) {
    const path = paths[index];
    if (cancelled.has(batchId)) {
      const outcome = { input: path, output: null, size_bytes: 0, ok: false, error: "Cancelled." };
      await emit("photos-export-progress", { batch_id: batchId, index, path, state: "failed", outcome });
      outcomes.push(outcome);
      continue;
    }
    await emit("photos-export-progress", { batch_id: batchId, index, path, state: "running" });
    await sleep(450);
    const dir = options.out_dir ?? path.slice(0, path.lastIndexOf("/"));
    const stem = path.split("/").pop()!.replace(/\.[^.]+$/, "");
    const outcome = { input: path, output: `${dir}/${stem}.${ext}`, size_bytes: 640_000, ok: true, error: null };
    await emit("photos-export-progress", { batch_id: batchId, index, path, state: "done", outcome });
    outcomes.push(outcome);
  }
  cancelled.delete(batchId);
  return outcomes;
}

function duplicates(args: Args) {
  const groups = [
    { kind: "exact", keep: "/preview/Pictures/Family/birthday.jpg", paths: ["/preview/Pictures/Family/birthday.jpg", "/preview/Pictures/Family/birthday (1).jpg"], bytes: 1_200_000 },
    { kind: "similar", keep: "/preview/Pictures/Trip/IMG_0421.HEIC", paths: ["/preview/Pictures/Trip/IMG_0421.HEIC", "/preview/Pictures/Trip/IMG_0422.HEIC"], bytes: 2_380_000 },
  ];
  const wide = Number(args.maxDistance) > 6;
  return groups.filter((g) => g.kind === "exact" || wide).filter((g) => g.paths.every((p) => present.has(p)));
}

export const photosFixtures: Record<string, (args: Args) => unknown> = {
  photos_folders: () => ({ folders: folders.map((path) => ({ path, exists: true })), chosen }),
  photos_set_folders: (args) => {
    folders = (args.folders as string[]) ?? [];
    chosen = true;
    present = new Set(SEEDS.filter((s) => folders.some((f) => s[0].startsWith(f + "/"))).map((s) => s[0]));
    return { folders: folders.map((path) => ({ path, exists: true })), chosen };
  },
  photos_index_start: index,
  photos_index_status: () => last,
  photos_index_cancel: () => null,
  photos_list: list,
  photos_duplicates: duplicates,
  photos_remove: (args) => report((args.paths as string[]) ?? [], false),
  photos_move: (args) => report((args.paths as string[]) ?? [], true),
  photos_edits_get: (args) => edits.get(String(args.path)) ?? null,
  photos_edits_set: (args) => {
    const e = args.edits as Edits | null;
    if (e) edits.set(String(args.path), e);
    else edits.delete(String(args.path));
    return null;
  },
  photos_preview: preview,
  photos_release: () => null,
  photos_export: runExport,
  photos_export_cancel: (args) => {
    cancelled.add(String(args.batchId));
    return null;
  },
};

export function photosDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/photos")) return undefined;
  if (options?.directory) return options.multiple ? ["/preview/Camera Uploads"] : "/preview/Exports";
  return null;
}
