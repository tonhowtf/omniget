import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;
type StemJob = {
  id: number;
  track_id: number | null;
  source: string;
  title: string;
  outputs: { stem: string; path: string; track_id: number | null }[];
  state: "queued" | "preparing" | "running" | "done" | "failed" | "cancelled";
  progress: number;
  error: string | null;
};

const MODEL_BYTES = 78_636_679;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let downloaded = false;
let nextId = 1;
let jobs: StemJob[] = [];
const cancelled = new Set<number>();

const sets = [
  {
    source: "/preview/Music/Aurora - Running Up That Hill.mp3",
    source_id: 1,
    vocals: "/preview/Music/Aurora - Running Up That Hill (Vocals).flac",
    vocals_id: 2,
    instrumental: "/preview/Music/Aurora - Running Up That Hill (Instrumental).flac",
    instrumental_id: 3,
    model: "spleeter-2stems",
    created_at: 1_762_430_000_000,
  },
];

function status() {
  return {
    runtime_installed: true,
    runtime_can_download: true,
    models: [
      {
        id: "spleeter-2stems",
        label: "Spleeter 2 stems (voice + instrumental)",
        bytes: MODEL_BYTES,
        license: "MIT",
        source: "https://github.com/deezer/spleeter",
        stems: ["vocals", "instrumental"],
        downloaded,
        default: true,
      },
    ],
  };
}

function stem(path: string): string {
  const name = path.split("/").pop() ?? path;
  return name.replace(/\.[^.]+$/, "");
}

function outputs(source: string, ext: string, dir: string, labels: { vocals: string; instrumental: string }) {
  const folder = dir || source.slice(0, source.lastIndexOf("/"));
  const base = `${folder}/${stem(source)}`;
  return [
    { stem: "vocals", path: `${base} (${labels.vocals}).${ext}`, track_id: null },
    { stem: "instrumental", path: `${base} (${labels.instrumental}).${ext}`, track_id: null },
  ];
}

async function installModel() {
  for (let step = 1; step <= 10; step += 1) {
    await sleep(150);
    await emit("music-stems-model-progress", { id: "spleeter-2stems", stage: "model", done: (MODEL_BYTES * step) / 10, total: MODEL_BYTES });
  }
  downloaded = true;
  return status();
}

async function publish(job: StemJob, patch: Partial<StemJob>) {
  Object.assign(job, patch);
  await emit("music-stems-progress", { ...job });
}

async function run(queue: StemJob[]) {
  for (const job of queue) {
    if (cancelled.has(job.id)) continue;
    await publish(job, { state: "preparing" });
    await sleep(300);
    for (let step = 1; step <= 10; step += 1) {
      if (cancelled.has(job.id)) {
        await publish(job, { state: "cancelled" });
        break;
      }
      await sleep(250);
      await publish(job, { state: "running", progress: step / 10 });
    }
    if (job.state === "running") await publish(job, { state: "done", progress: 1 });
  }
}

export const stemsFixtures: Record<string, (args: Args) => unknown> = {
  music_stems_status: () => status(),
  music_stems_install_model: () => installModel(),
  music_stems_remove_model: () => {
    downloaded = false;
    return status();
  },
  music_stems_start: ({ request }) => {
    const r = (request ?? {}) as { paths?: string[]; track_ids?: number[]; format?: string; out_dir?: string; labels?: { vocals: string; instrumental: string } };
    const labels = r.labels ?? { vocals: "Vocals", instrumental: "Instrumental" };
    const sources = [...(r.track_ids ?? []).map((id) => ({ id, path: `/preview/Music/Track ${id}.mp3` })), ...(r.paths ?? []).map((path) => ({ id: null, path }))];
    const created = sources.map(({ id, path }) => ({
      id: nextId++,
      track_id: id,
      source: path,
      title: stem(path),
      outputs: outputs(path, r.format ?? "flac", r.out_dir ?? "", labels),
      state: "queued" as const,
      progress: 0,
      error: null,
    }));
    jobs = [...jobs, ...created];
    void run(created);
    return created.map((job) => ({ ...job }));
  },
  music_stems_cancel: ({ id }) => {
    for (const job of jobs) if (id == null || job.id === id) cancelled.add(job.id);
    return null;
  },
  music_stems_jobs: () => jobs.map((job) => ({ ...job })),
  music_stems_clear: () => {
    jobs = jobs.filter((job) => job.state === "queued" || job.state === "preparing" || job.state === "running");
    return null;
  },
  music_stems_sets: () => sets,
};
