export type LottieInfo = {
  name: string;
  version: string;
  fps: number;
  inPoint: number;
  outPoint: number;
  frames: number;
  width: number;
  height: number;
  durationMs: number;
  layers: number;
  assets: number;
  images: number;
  is3d: boolean;
};

export type LottieCheck = { ok: true; data: Record<string, unknown>; info: LottieInfo } | { ok: false; error: "not_json" | "not_lottie" | "no_frames" };

function n(value: unknown, fallback = 0): number {
  const v = Number(value);
  return Number.isFinite(v) ? v : fallback;
}

export function readLottie(text: string): LottieCheck {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return { ok: false, error: "not_json" };
  }
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return { ok: false, error: "not_lottie" };
  const data = raw as Record<string, unknown>;
  if (!Array.isArray(data.layers) || !("w" in data) || !("h" in data) || !("fr" in data)) return { ok: false, error: "not_lottie" };
  const fps = n(data.fr, 30);
  const inPoint = n(data.ip);
  const outPoint = n(data.op);
  const frames = Math.max(0, outPoint - inPoint);
  if (frames <= 0 || fps <= 0) return { ok: false, error: "no_frames" };
  const assets = Array.isArray(data.assets) ? (data.assets as Record<string, unknown>[]) : [];
  const images = assets.filter((a) => a && typeof a === "object" && typeof a.p === "string" && !Array.isArray(a.layers)).length;
  return {
    ok: true,
    data,
    info: {
      name: typeof data.nm === "string" ? data.nm : "",
      version: typeof data.v === "string" ? data.v : "",
      fps,
      inPoint,
      outPoint,
      frames,
      width: n(data.w),
      height: n(data.h),
      durationMs: (frames / fps) * 1000,
      layers: (data.layers as unknown[]).length,
      assets: assets.length,
      images,
      is3d: n(data.ddd) === 1,
    },
  };
}

export function frameLabel(frame: number, total: number): string {
  return `${Math.round(frame)} / ${Math.round(total)}`;
}

export function timeLabel(ms: number): string {
  const s = Math.max(0, ms) / 1000;
  return s.toFixed(2);
}

export function pngName(base: string, frame: number): string {
  const stem = base.replace(/\.(json|lottie)$/i, "").replace(/[\\/:*?"<>|]+/g, "-").trim() || "animation";
  return `${stem}-frame-${Math.round(frame)}.png`;
}
