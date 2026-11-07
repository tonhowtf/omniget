import type { Container, Quality, Region, RecordOptions, Sources } from "./types";

export const FPS_CHOICES = [24, 30, 60] as const;
export const CONTAINERS: Container[] = ["mp4", "mov", "mkv"];
export const QUALITIES: Quality[] = ["high", "balanced", "small"];
export const COUNTDOWN_SECONDS = 3;

export const ERROR_CODES = [
  "CAPTURE_NO_FFMPEG",
  "CAPTURE_NO_SCREENS",
  "CAPTURE_NO_DISPLAY",
  "CAPTURE_PERMISSION_SCREEN",
  "CAPTURE_PERMISSION_MIC",
  "CAPTURE_DEVICE_BUSY",
  "CAPTURE_RECORD_FAILED",
  "CAPTURE_EMPTY_OUTPUT",
  "CAPTURE_ALREADY_RECORDING",
  "CAPTURE_NOT_RECORDING",
  "CAPTURE_BAD_OPTIONS",
  "CAPTURE_NO_SEGMENTS",
  "CAPTURE_CANCELLED",
  "CAPTURE_EXPORT_FAILED",
  "CAPTURE_PROBE_FAILED",
  "CAPTURE_NOT_FOUND",
  "CAPTURE_OUT_DIR",
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number] | "UNKNOWN";

export type ParsedError = { code: ErrorCode; detail: string };

export function parseError(text: string | null | undefined): ParsedError {
  const raw = (text ?? "").trim();
  const match = /^(CAPTURE_[A-Z_]+)(?::\s*([\s\S]*))?$/.exec(raw);
  if (match && (ERROR_CODES as readonly string[]).includes(match[1])) {
    return { code: match[1] as ErrorCode, detail: (match[2] ?? "").trim() };
  }
  return { code: "UNKNOWN", detail: raw };
}

export function errorKey(code: ErrorCode): string {
  return code === "UNKNOWN" ? "capture.err_unknown" : `capture.err_${code.slice("CAPTURE_".length).toLowerCase()}`;
}

export function isPermission(code: ErrorCode): boolean {
  return code === "CAPTURE_PERMISSION_SCREEN" || code === "CAPTURE_PERMISSION_MIC";
}

export type RegionDraft = { x: string; y: string; w: string; h: string };

export type RegionCheck = { region: Region | null; problem: "incomplete" | "not_integer" | "too_small" | null };

export function checkRegion(draft: RegionDraft): RegionCheck {
  const vals = [draft.x, draft.y, draft.w, draft.h].map((v) => String(v ?? "").trim());
  if (vals.some((v) => v === "")) return { region: null, problem: "incomplete" };
  if (vals.some((v) => !/^\d+$/.test(v))) return { region: null, problem: "not_integer" };
  const [x, y, w, h] = vals.map(Number);
  if (w < 16 || h < 16) return { region: null, problem: "too_small" };
  return { region: { x, y, w, h }, problem: null };
}

export function pickDefaultMic(sources: Sources | null): string | null {
  const list = sources?.audio ?? [];
  if (list.length === 0) return null;
  const mic = list.find((a) => /\b(microphone|mic|mikrofon|microfone|micrófono|microphone array|input)\b/i.test(a.name) && !/monitor/i.test(a.name));
  return (mic ?? list[0]).id;
}

export function buildOptions(input: {
  screen: string;
  useRegion: boolean;
  region: RegionDraft;
  fps: number;
  micOn: boolean;
  mic: string | null;
  cursor: boolean;
  container: Container;
  quality: Quality;
  outDir: string | null;
}): { options: RecordOptions | null; problem: RegionCheck["problem"] | "no_screen" | "no_mic" } {
  if (!input.screen) return { options: null, problem: "no_screen" };
  if (input.micOn && !input.mic) return { options: null, problem: "no_mic" };
  let region: Region | null = null;
  if (input.useRegion) {
    const checked = checkRegion(input.region);
    if (!checked.region) return { options: null, problem: checked.problem };
    region = checked.region;
  }
  return {
    options: {
      screen: input.screen,
      region,
      fps: input.fps,
      mic: input.micOn ? input.mic : null,
      cursor: input.cursor,
      container: input.container,
      quality: input.quality,
      out_dir: input.outDir && input.outDir.trim() ? input.outDir : null,
    },
    problem: null,
  };
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function folderOf(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : path;
}
