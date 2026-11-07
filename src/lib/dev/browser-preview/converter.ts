import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;
type Category = "data" | "document" | "image" | "cad" | "audio" | "video";
type Engine = "native" | "ffmpeg" | "pandoc" | "imagemagick" | "libreoffice";

const TOOLS = { ffmpeg: true, pandoc: false, imagemagick: true, libreoffice: false };

const FORMATS: { id: string; label: string; category: Category; extensions: string[] }[] = [
  { id: "json", label: "JSON", category: "data", extensions: ["json"] },
  { id: "yaml", label: "YAML", category: "data", extensions: ["yaml", "yml"] },
  { id: "toml", label: "TOML", category: "data", extensions: ["toml"] },
  { id: "csv", label: "CSV", category: "data", extensions: ["csv"] },
  { id: "xml", label: "XML", category: "data", extensions: ["xml"] },
  { id: "md", label: "Markdown", category: "document", extensions: ["md", "markdown"] },
  { id: "html", label: "HTML", category: "document", extensions: ["html", "htm"] },
  { id: "txt", label: "Text", category: "document", extensions: ["txt"] },
  { id: "docx", label: "DOCX", category: "document", extensions: ["docx"] },
  { id: "pdf", label: "PDF", category: "document", extensions: ["pdf"] },
  { id: "png", label: "PNG", category: "image", extensions: ["png"] },
  { id: "jpeg", label: "JPEG", category: "image", extensions: ["jpg", "jpeg"] },
  { id: "webp", label: "WebP", category: "image", extensions: ["webp"] },
  { id: "gif", label: "GIF", category: "image", extensions: ["gif"] },
  { id: "avif", label: "AVIF", category: "image", extensions: ["avif"] },
  { id: "tiff", label: "TIFF", category: "image", extensions: ["tiff", "tif"] },
  { id: "svg", label: "SVG", category: "image", extensions: ["svg"] },
  { id: "heic", label: "HEIC", category: "image", extensions: ["heic", "heif", "hif"] },
  { id: "jxl", label: "JPEG XL", category: "image", extensions: ["jxl"] },
  { id: "psd", label: "Photoshop", category: "image", extensions: ["psd", "psb"] },
  { id: "raw", label: "Camera RAW", category: "image", extensions: ["cr2", "cr3", "nef", "nrw", "arw", "dng", "raf", "orf", "rw2", "pef", "srw"] },
  { id: "layers", label: "PNG layers", category: "image", extensions: [] },
  { id: "dxf", label: "DXF", category: "cad", extensions: ["dxf"] },
  { id: "dwg", label: "DWG", category: "cad", extensions: ["dwg"] },
  { id: "mp3", label: "MP3", category: "audio", extensions: ["mp3"] },
  { id: "flac", label: "FLAC", category: "audio", extensions: ["flac"] },
  { id: "wav", label: "WAV", category: "audio", extensions: ["wav"] },
  { id: "mp4", label: "MP4", category: "video", extensions: ["mp4"] },
  { id: "webm", label: "WebM", category: "video", extensions: ["webm"] },
  { id: "mov", label: "MOV", category: "video", extensions: ["mov"] },
];

const EDGES: [string, string, Engine, boolean][] = [
  ["json", "yaml", "native", false], ["json", "toml", "native", true], ["json", "csv", "native", true], ["json", "xml", "native", false],
  ["yaml", "json", "native", false], ["csv", "json", "native", false], ["xml", "json", "native", false], ["toml", "json", "native", false],
  ["md", "html", "native", false], ["md", "txt", "native", true], ["html", "md", "native", true], ["html", "txt", "native", true],
  ["md", "docx", "pandoc", false], ["docx", "md", "pandoc", true], ["docx", "pdf", "libreoffice", false], ["md", "pdf", "pandoc", false],
  ["png", "jpeg", "native", true], ["png", "webp", "native", false], ["png", "gif", "native", true], ["png", "avif", "imagemagick", true],
  ["jpeg", "png", "native", false], ["jpeg", "webp", "native", true], ["jpeg", "avif", "imagemagick", true],
  ["webp", "png", "native", false], ["webp", "jpeg", "native", true],
  ["heic", "png", "native", false], ["heic", "jpeg", "native", true], ["heic", "webp", "native", false],
  ["jxl", "png", "native", false], ["jxl", "jpeg", "native", true], ["jxl", "webp", "native", false],
  ["psd", "png", "native", true], ["psd", "jpeg", "native", true], ["psd", "layers", "native", false],
  ["raw", "jpeg", "native", true], ["raw", "png", "native", true], ["raw", "tiff", "native", true],
  ["png", "svg", "native", true], ["jpeg", "svg", "native", true], ["webp", "svg", "native", true], ["svg", "png", "native", false],
  ["dwg", "dxf", "native", false], ["dxf", "svg", "native", true], ["dxf", "pdf", "native", true], ["dwg", "svg", "native", true], ["dwg", "pdf", "native", true],
  ["mp3", "flac", "ffmpeg", false], ["mp3", "wav", "ffmpeg", false], ["flac", "mp3", "ffmpeg", true], ["wav", "mp3", "ffmpeg", true],
  ["mov", "mp4", "ffmpeg", true], ["mov", "webm", "ffmpeg", true], ["mov", "gif", "ffmpeg", true], ["mov", "mp3", "ffmpeg", true],
  ["mp4", "webm", "ffmpeg", true], ["mp4", "gif", "ffmpeg", true], ["mp4", "mp3", "ffmpeg", true],
];

const SIZES: Record<string, number> = { heic: 2_910_000, psd: 38_600_000, dwg: 1_870_000, json: 18_400, md: 6_200, docx: 412_000, png: 2_480_000, jpeg: 1_240_000, mov: 84_000_000, mp3: 7_300_000 };

function detect(path: string) {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return FORMATS.find((f) => f.extensions.includes(ext)) ?? null;
}

function available(engine: Engine): boolean {
  return engine === "native" || TOOLS[engine];
}

function targetsFor(from: string) {
  return EDGES.filter(([a]) => a === from).map(([, to, engine, lossy]) => {
    const info = FORMATS.find((f) => f.id === to)!;
    const ok = available(engine);
    return {
      format: to,
      label: info.label,
      category: info.category,
      route: ok ? { steps: [{ from, to, engine, lossy }], lossy, engines: [engine] } : { steps: [], lossy: false, engines: [] },
      missing_tool: ok ? null : engine,
    };
  });
}

function inspectOne(path: string) {
  const name = path.split("/").pop() ?? path;
  const format = detect(path);
  if (name.startsWith("empty")) return { path, name, size_bytes: 0, format, targets: [], error: "The file is empty (0 bytes). Pick a file that has content." };
  return { path, name, size_bytes: SIZES[format?.id ?? ""] ?? 24_000, format, targets: format ? targetsFor(format.id) : [], error: null };
}

const SAMPLE_FILES = [
  "/preview/Documents/relatório trimestral 2026 – versão final revisada pelo jurídico.docx",
  "/preview/Documents/notes.md",
  "/preview/Data/config.json",
  "/preview/Pictures/screenshot.png",
  "/preview/Pictures/IMG_0421.HEIC",
  "/preview/Design/mockup.psd",
  "/preview/Projects/planta-baixa.dwg",
  "/preview/Pictures/写真_東京タワー.jpeg",
  "/preview/Movies/demo.mov",
  "/preview/Downloads/archive.xyz",
];

const cancelled = new Set<string>();
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function runBatch(args: Args) {
  const batchId = String(args.batchId);
  const items = (args.items as { path: string; target: string }[]) ?? [];
  const outDir = (args.outDir as string | null) ?? null;
  const outcomes = [];
  for (let index = 0; index < items.length; index++) {
    const { path, target } = items[index];
    const info = inspectOne(path);
    const from = info.format?.id ?? "";
    const edge = EDGES.find(([a, b]) => a === from && b === target);
    const route = edge ? { steps: [{ from, to: target, engine: edge[2], lossy: edge[3] }], lossy: edge[3], engines: [edge[2]] } : null;
    if (cancelled.has(batchId)) {
      outcomes.push({ input: path, output: null, size_bytes: 0, route, ok: false, error: "Cancelled.", skipped: false });
      continue;
    }
    for (const percent of [0, 25, 55, 80, 100]) {
      await emit("converter-progress", { batch_id: batchId, index, path, percent, state: "running" });
      await sleep(from === "mov" ? 420 : 180);
      if (cancelled.has(batchId)) break;
    }
    const ext = FORMATS.find((f) => f.id === target)?.extensions[0] ?? target;
    const dir = outDir ?? path.slice(0, path.lastIndexOf("/"));
    const stem = info.name.replace(/\.[^.]+$/, "");
    const output = target === "layers" ? `${dir}/${stem} layers` : `${dir}/${stem}.${ext}`;
    const fail = from === "mov" && target === "webm";
    const outcome = cancelled.has(batchId)
      ? { input: path, output: null, size_bytes: 0, route, ok: false, error: "Cancelled.", skipped: false }
      : fail
        ? { input: path, output: null, size_bytes: 0, route, ok: false, error: "ffmpeg stopped: the video stream uses a codec this build cannot decode (ProRes RAW).", skipped: false }
        : { input: path, output, size_bytes: Math.round(info.size_bytes * 0.6), route, ok: true, error: null, skipped: false };
    await emit("converter-progress", { batch_id: batchId, index, path, percent: 100, state: outcome.ok ? "done" : "failed", outcome });
    outcomes.push(outcome);
  }
  cancelled.delete(batchId);
  return outcomes;
}

export const converterFixtures: Record<string, (args: Args) => unknown> = {
  converter_tools: () => ({ ...TOOLS }),
  converter_formats: () => FORMATS,
  converter_inspect: ({ paths }) => ((paths as string[]) ?? []).map(inspectOne),
  converter_run: runBatch,
  converter_cancel: ({ batchId }) => {
    cancelled.add(String(batchId));
    return null;
  },
  reveal_file: () => null,
};

export function converterDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/converter")) return undefined;
  if (options?.directory) return "/preview/Converted";
  return SAMPLE_FILES;
}
