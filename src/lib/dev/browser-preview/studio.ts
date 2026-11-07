import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

const VIDEO = "/preview/Movies/founder interview – take 3.mp4";
const BROKEN = "/preview/Movies/broken-export.mp4";
const SUBS = "/preview/Subtitles/founder interview.srt";

const MODELS = [
  { id: "whisper-tiny", label: "Whisper tiny (fast, ~45 MB)", bytes: 41_884_587, default: false },
  { id: "whisper-base", label: "Whisper base (balanced, ~80 MB)", bytes: 77_935_045, default: true },
  { id: "whisper-small", label: "Whisper small (accurate, ~250 MB)", bytes: 250_114_482, default: false },
];

const downloaded = new Set<string>(["whisper-tiny"]);
const cancelled = new Set<string>();
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const SPEECH: [number, number, string][] = [
  [400, 3200, "So the idea started on a weekend, honestly by accident."],
  [3400, 6900, "We kept downloading the same lecture videos over and over."],
  [7100, 10400, "And every single time the captions were out of sync."],
  [10800, 14600, "So we built a tool that does it locally, on your own machine."],
  [15000, 18200, "No uploads, no waiting in a queue, no monthly bill."],
  [18600, 22400, "Now it reframes the clip for vertical and burns the captions in."],
  [22800, 26500, "Which is exactly what you are watching right now."],
];

function status() {
  return {
    runtime_installed: true,
    runtime_downloadable: true,
    default_model: "whisper-base",
    ready: downloaded.has("whisper-base"),
    models: MODELS.map((m) => ({ ...m, downloaded: downloaded.has(m.id) })),
  };
}

function probe(args: Args) {
  const path = String(args.path ?? "");
  if (path.includes("broken")) throw "FFmpeg could not read this file. It may be damaged or not a video.";
  const name = path.split("/").pop() ?? path;
  return { path, name, size_bytes: 184_320_000, duration_ms: 28_400, width: 1920, height: 1080, fps: 29.97, has_video: true, has_audio: !name.includes("silent"), video_codec: "h264" };
}

async function install(args: Args) {
  const model = String(args.model ?? "whisper-base");
  const spec = MODELS.find((m) => m.id === model) ?? MODELS[1];
  const parts = [
    { id: `${spec.id}:encoder.onnx`, total: Math.round(spec.bytes * 0.3) },
    { id: `${spec.id}:decoder_merged.onnx`, total: Math.round(spec.bytes * 0.69) },
  ];
  for (const part of parts) {
    for (let step = 0; step <= 10; step++) {
      await emit("studio-asr-progress", { model: spec.id, id: part.id, stage: "download", done: Math.round((part.total * step) / 10), total: part.total });
      await sleep(110);
    }
  }
  downloaded.add(spec.id);
  return status();
}

async function transcribe(args: Args) {
  const jobId = String(args.jobId);
  for (let p = 0; p <= 95; p += 7) {
    await emit("studio-transcribe-progress", { job_id: jobId, percent: p });
    await sleep(120);
  }
  return { lang: "en", model: String(args.model ?? "whisper-base"), segments: SPEECH.map(([start_ms, end_ms, text]) => ({ start_ms, end_ms, text })) };
}

let renders = 0;

async function render(args: Args) {
  const jobId = String(args.jobId);
  const req = (args.request ?? {}) as { input?: string; out_dir?: string | null; reframe?: { aspect?: string; mode?: string } };
  await emit("studio-render-progress", { job_id: jobId, percent: 0, stage: "preparing" });
  await sleep(500);
  for (let p = 0; p <= 99; p += 3) {
    if (cancelled.has(jobId)) {
      cancelled.delete(jobId);
      throw "Cancelled.";
    }
    await emit("studio-render-progress", { job_id: jobId, percent: p, stage: "rendering" });
    await sleep(90);
  }
  await emit("studio-render-progress", { job_id: jobId, percent: 100, stage: "done" });
  renders += 1;
  const input = req.input ?? VIDEO;
  const dir = req.out_dir ?? input.slice(0, input.lastIndexOf("/"));
  const stem = (input.split("/").pop() ?? "video").replace(/\.[^.]+$/, "");
  const sizes: Record<string, [number, number]> = { "9:16": req.reframe?.mode === "fit" ? [1080, 1920] : [606, 1080], "1:1": [1080, 1080], "4:5": req.reframe?.mode === "fit" ? [1080, 1350] : [864, 1080] };
  const [width, height] = sizes[req.reframe?.aspect ?? ""] ?? [1920, 1080];
  return { output: `${dir}/${stem} (studio${renders > 1 ? ` ${renders}` : ""}).mp4`, size_bytes: 21_450_000, duration_ms: 28_400, width, height };
}

const SAMPLE_SRT = SPEECH.map(([s, e, text], i) => {
  const f = (ms: number) => new Date(ms).toISOString().slice(11, 23).replace(".", ",");
  return `${i + 1}\n${f(s)} --> ${f(e)}\n${text}\n`;
}).join("\n");

export const studioFixtures: Record<string, (args: Args) => unknown> = {
  studio_tools: () => ({ ffmpeg: true, subtitles: true, drawtext: true }),
  studio_probe: probe,
  studio_asr_status: status,
  studio_asr_install: install,
  studio_transcribe: transcribe,
  studio_render: render,
  studio_cancel: ({ jobId }) => {
    cancelled.add(String(jobId));
    return null;
  },
  studio_save_text: () => null,
  studio_read_text: () => SAMPLE_SRT,
  open_path_default: () => null,
  reveal_file: () => null,
};

function onStudio(): boolean {
  return location.pathname.startsWith("/studio");
}

export function studioDialog(options: Record<string, unknown> | undefined): unknown {
  if (!onStudio()) return undefined;
  if (options?.directory) return "/preview/Exports";
  const filters = (options?.filters as { extensions?: string[] }[] | undefined) ?? [];
  if (filters.some((f) => f.extensions?.includes("srt"))) return SUBS;
  if (location.search.includes("broken")) return BROKEN;
  return VIDEO;
}

export function studioSaveDialog(options: Record<string, unknown> | undefined): unknown {
  if (!onStudio()) return undefined;
  return typeof options?.defaultPath === "string" ? options.defaultPath : "/preview/Subtitles/captions.srt";
}
