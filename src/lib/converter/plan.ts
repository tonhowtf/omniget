import type { Category, Engine, ExternalTool, FormatInfo, InspectResult, Route, TargetOption, VectorPreset } from "./types";

export const CATEGORY_ORDER: Category[] = ["data", "document", "image", "cad", "audio", "video"];

export const VECTOR_PRESETS: VectorPreset[] = ["colors", "bw", "lineart"];

const TRACEABLE = new Set(["png", "jpeg", "webp"]);

export function tracesToSvg(route: Route): boolean {
  return route.steps.some((s) => s.to === "svg" && TRACEABLE.has(s.from));
}

export const EXTERNAL_TOOLS: ExternalTool[] = ["ffmpeg", "pandoc", "imagemagick", "libreoffice"];

export const ENGINE_NAMES: Record<ExternalTool, string> = {
  ffmpeg: "ffmpeg",
  pandoc: "Pandoc",
  imagemagick: "ImageMagick",
  libreoffice: "LibreOffice",
};

export function engineName(engine: Engine, nativeLabel: string): string {
  return engine === "native" ? nativeLabel : ENGINE_NAMES[engine];
}

export function isReadable(file: InspectResult): file is InspectResult & { format: FormatInfo } {
  return !file.error && !!file.format;
}

export function isAvailable(option: TargetOption): boolean {
  return !option.missing_tool && option.route.steps.length > 0;
}

export type CommonTarget = {
  format: string;
  label: string;
  category: Category;
  available: boolean;
  missingTool: Engine | null;
  lossy: boolean;
  skipped: number;
};

export function commonTargets(files: InspectResult[]): CommonTarget[] {
  const readable = files.filter(isReadable);
  if (readable.length === 0) return [];
  const seen = new Map<string, { label: string; category: Category; rank: number }>();
  let rank = 0;
  for (const file of readable) {
    for (const option of file.targets) {
      if (!seen.has(option.format)) seen.set(option.format, { label: option.label, category: option.category, rank: rank++ });
    }
  }
  const out: (CommonTarget & { rank: number })[] = [];
  for (const [format, meta] of seen) {
    let skipped = 0;
    let missingTool: Engine | null = null;
    let lossy = false;
    let reachable = true;
    for (const file of readable) {
      if (file.format.id === format) {
        skipped += 1;
        continue;
      }
      const option = file.targets.find((o) => o.format === format);
      if (!option) {
        reachable = false;
        break;
      }
      if (!isAvailable(option)) missingTool ??= option.missing_tool ?? null;
      else if (option.route.lossy) lossy = true;
    }
    if (!reachable || skipped === readable.length) continue;
    out.push({ format, label: meta.label, category: meta.category, available: missingTool === null, missingTool, lossy, skipped, rank: meta.rank });
  }
  out.sort((a, b) => CATEGORY_ORDER.indexOf(a.category) - CATEGORY_ORDER.indexOf(b.category) || a.rank - b.rank);
  return out.map(({ rank: _rank, ...rest }) => rest);
}

export function groupByCategory<T extends { category: Category }>(items: T[]): { category: Category; items: T[] }[] {
  return CATEGORY_ORDER.map((category) => ({ category, items: items.filter((item) => item.category === category) })).filter(
    (group) => group.items.length > 0,
  );
}

export type RowPlan =
  | { kind: "unreadable"; error: string }
  | { kind: "unknown" }
  | { kind: "none" }
  | { kind: "skip"; format: FormatInfo }
  | { kind: "unsupported"; target: string }
  | { kind: "missing"; tool: Engine; option: TargetOption }
  | { kind: "convert"; option: TargetOption };

export function planRow(file: InspectResult, target: string | null): RowPlan {
  if (file.error) return { kind: "unreadable", error: file.error };
  if (!file.format) return { kind: "unknown" };
  if (!target) return { kind: "none" };
  if (file.format.id === target) return { kind: "skip", format: file.format };
  const option = file.targets.find((o) => o.format === target);
  if (!option) return { kind: "unsupported", target };
  if (!isAvailable(option)) return { kind: "missing", tool: option.missing_tool ?? "native", option };
  return { kind: "convert", option };
}

export function labeler(formats: FormatInfo[]): (id: string) => string {
  const map = new Map(formats.map((f) => [f.id, f.label]));
  return (id) => map.get(id) ?? id.toUpperCase();
}

export function routeEngines(route: Route): Engine[] {
  const list = route.engines?.length ? route.engines : route.steps.map((s) => s.engine);
  return [...new Set(list)];
}

export function routeLabel(route: Route, labelOf: (id: string) => string, nativeLabel: string): string {
  if (route.steps.length === 0) return "";
  const chain = [route.steps[0].from, ...route.steps.map((s) => s.to)].map(labelOf).join(" → ");
  const engines = routeEngines(route).map((e) => engineName(e, nativeLabel)).join(", ");
  return engines ? `${chain} · ${engines}` : chain;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

const TOOL_HINTS: [RegExp, ExternalTool][] = [
  [/ffmpeg|ffprobe/i, "ffmpeg"],
  [/pandoc/i, "pandoc"],
  [/imagemagick|\bmagick\b/i, "imagemagick"],
  [/libreoffice|soffice/i, "libreoffice"],
];

export type FailureFix = { kind: "missing_tool"; tool: ExternalTool } | { kind: "generic" };

export function failureFix(error: string | null | undefined): FailureFix {
  if (!error) return { kind: "generic" };
  if (!/not installed|not found|missing|install/i.test(error)) return { kind: "generic" };
  for (const [pattern, tool] of TOOL_HINTS) {
    if (pattern.test(error)) return { kind: "missing_tool", tool };
  }
  return { kind: "generic" };
}

export type RowStatus = "idle" | "queued" | "running" | "done" | "failed" | "skipped" | "stopped";

export function tally(statuses: RowStatus[]) {
  const count = (s: RowStatus) => statuses.filter((x) => x === s).length;
  return {
    total: statuses.length,
    done: count("done"),
    failed: count("failed"),
    skipped: count("skipped"),
    running: count("running"),
    queued: count("queued"),
    stopped: count("stopped"),
  };
}

export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function parentDir(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx > 0 ? path.slice(0, idx) : path;
}

export function mergePaths(current: string[], incoming: string[]): string[] {
  const seen = new Set(current);
  const out = [...current];
  for (const path of incoming) {
    if (!seen.has(path)) {
      seen.add(path);
      out.push(path);
    }
  }
  return out;
}
