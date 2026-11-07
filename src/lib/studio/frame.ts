import type { Aspect, FrameMode } from "./types";

const MAX_BASE = 2160;

export function even(v: number): number {
  const n = Math.floor(Math.max(2, v));
  return Math.max(2, n - (n % 2));
}

export function aspectRatio(aspect: Aspect): [number, number] | null {
  switch (aspect) {
    case "9:16":
      return [9, 16];
    case "1:1":
      return [1, 1];
    case "4:5":
      return [4, 5];
    default:
      return null;
  }
}

export type Crop = { w: number; h: number; x: number; y: number };

export function cropRect(srcW: number, srcH: number, aw: number, ah: number, position: number): Crop {
  const target = aw / ah;
  let w: number;
  let h: number;
  if (srcW / srcH > target) {
    w = even(srcH * target);
    h = even(srcH);
  } else {
    w = even(srcW);
    h = even(srcW / target);
  }
  w = Math.min(w, Math.max(2, srcW));
  h = Math.min(h, Math.max(2, srcH));
  const pos = Number.isFinite(position) ? Math.min(1, Math.max(0, position)) : 0.5;
  return { w, h, x: Math.round((srcW - w) * pos), y: Math.floor((srcH - h) / 2) };
}

export function outputSize(srcW: number, srcH: number, aspect: Aspect, mode: FrameMode): [number, number] {
  const ratio = aspectRatio(aspect);
  if (!ratio) return [even(srcW), even(srcH)];
  const [aw, ah] = ratio;
  if (mode === "crop") {
    const c = cropRect(srcW, srcH, aw, ah, 0.5);
    return [c.w, c.h];
  }
  const base = Math.min(srcW, srcH, MAX_BASE);
  return aw <= ah ? [even(base), even((base * ah) / aw)] : [even((base * aw) / ah), even(base)];
}

export function hasCropSlack(srcW: number, srcH: number, aspect: Aspect): boolean {
  const ratio = aspectRatio(aspect);
  if (!ratio) return false;
  return srcW / srcH > ratio[0] / ratio[1] + 0.001;
}

export function blurRadius(outW: number, outH: number): number {
  return Math.min(60, Math.max(4, Math.floor(Math.min(outW, outH) / 40)));
}

export type IntroLayout = { titleSize: number; subtitleSize: number; titleYs: number[]; subtitleYs: number[] };

export function introLayout(width: number, height: number, titles: number, subtitles: number): IntroLayout {
  const short = Math.min(width, height);
  const titleSize = Math.max(8, Math.round(short * 0.085));
  const subtitleSize = Math.max(6, Math.round(short * 0.045));
  const titleLh = titleSize * 1.2;
  const subtitleLh = subtitleSize * 1.3;
  const gap = titles > 0 && subtitles > 0 ? subtitleSize * 0.8 : 0;
  const block = titles * titleLh + gap + subtitles * subtitleLh;
  const top = Math.max(0, (height - block) / 2);
  const titleYs = Array.from({ length: titles }, (_, i) => Math.round(top + i * titleLh + (titleLh - titleSize) / 2));
  const subTop = top + titles * titleLh + gap;
  const subtitleYs = Array.from({ length: subtitles }, (_, i) => Math.round(subTop + i * subtitleLh + (subtitleLh - subtitleSize) / 2));
  return { titleSize, subtitleSize, titleYs, subtitleYs };
}

export function charsPerLine(width: number, fontSize: number): number {
  return Math.max(6, Math.floor((width * 0.86) / (fontSize * 0.56)));
}

export function wrapText(text: string, maxChars: number, maxLines: number): string[] {
  const words = text.replace(/[\u0000-\u001f]+/g, " ").split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let current = "";
  for (const word of words) {
    const next = current ? `${current} ${word}` : word;
    if (current && next.length > maxChars) {
      lines.push(current);
      current = word;
    } else {
      current = next;
    }
  }
  if (current) lines.push(current);
  if (lines.length <= maxLines) return lines;
  const kept = lines.slice(0, maxLines);
  kept[maxLines - 1] = `${lines.slice(maxLines - 1).join(" ").slice(0, Math.max(1, maxChars - 1)).trimEnd()}…`;
  return kept;
}

export function introLines(width: number, height: number, title: string, subtitle: string): { title: string[]; subtitle: string[] } {
  const short = Math.min(width, height);
  const t = wrapText(title, charsPerLine(width, Math.round(short * 0.085)), 3);
  const s = wrapText(subtitle, charsPerLine(width, Math.round(short * 0.045)), 2);
  return { title: t, subtitle: s };
}
