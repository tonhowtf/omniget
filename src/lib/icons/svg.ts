import type { IconData } from "./types";

export type Transform = { rotate: number; hFlip: boolean; vFlip: boolean };

export type BuildOptions = {
  size?: number;
  color?: string | null;
  strokeWidth?: number | null;
  rotate?: number;
  hFlip?: boolean;
  vFlip?: boolean;
};

export type SvgParts = { body: string; viewBox: string; width: number; height: number };

const SVG_NS = "http://www.w3.org/2000/svg";

export const MIN_SIZE = 8;
export const MAX_SIZE = 1024;
export const DEFAULT_SIZE = 24;

export function clampSize(value: unknown): number {
  const n = typeof value === "number" ? value : Number(value);
  if (value === null || value === "" || !Number.isFinite(n)) return DEFAULT_SIZE;
  return Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.round(n)));
}

export function fmt(n: number): string {
  if (!Number.isFinite(n)) return "0";
  return String(Math.round(n * 1000) / 1000);
}

export function safeColor(color: string | null | undefined): string | null {
  if (!color) return null;
  const c = color.trim();
  if (c === "currentColor") return null;
  if (/^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(c)) return c.toLowerCase();
  if (/^[a-z]{3,20}$/i.test(c)) return c.toLowerCase();
  return null;
}

export function hasStroke(body: string): boolean {
  return /stroke-width\s*=\s*["'][^"']*["']/.test(body);
}

export function rewriteStroke(body: string, width: number | null | undefined): string {
  if (width == null || !Number.isFinite(width) || width <= 0) return body;
  return body.replace(/stroke-width\s*=\s*(["'])[^"']*\1/g, `stroke-width="${fmt(width)}"`);
}

export function recolor(body: string, color: string | null | undefined): string {
  const c = safeColor(color);
  return c ? body.replace(/currentColor/g, c) : body;
}

export function combine(icon: Pick<IconData, "rotate" | "h_flip" | "v_flip">, extra?: Partial<Transform>): Transform {
  return {
    rotate: (((icon.rotate + (extra?.rotate ?? 0)) % 4) + 4) % 4,
    hFlip: icon.h_flip !== Boolean(extra?.hFlip),
    vFlip: icon.v_flip !== Boolean(extra?.vFlip),
  };
}

export function applyTransform(icon: Pick<IconData, "body" | "width" | "height" | "left" | "top">, t: Transform): SvgParts {
  let { left, top, width, height } = icon;
  let rotate = t.rotate;
  const transforms: string[] = [];
  if (t.hFlip) {
    if (t.vFlip) rotate += 2;
    else {
      transforms.push(`translate(${fmt(width + left)} ${fmt(-top)})`, "scale(-1 1)");
      left = top = 0;
    }
  } else if (t.vFlip) {
    transforms.push(`translate(${fmt(-left)} ${fmt(height + top)})`, "scale(1 -1)");
    left = top = 0;
  }
  rotate = ((rotate % 4) + 4) % 4;
  if (rotate === 1) {
    const c = height / 2 + top;
    transforms.unshift(`rotate(90 ${fmt(c)} ${fmt(c)})`);
  } else if (rotate === 2) {
    transforms.unshift(`rotate(180 ${fmt(width / 2 + left)} ${fmt(height / 2 + top)})`);
  } else if (rotate === 3) {
    const c = width / 2 + left;
    transforms.unshift(`rotate(-90 ${fmt(c)} ${fmt(c)})`);
  }
  if (rotate % 2 === 1) {
    if (left !== top) [left, top] = [top, left];
    [width, height] = [height, width];
  }
  const body = transforms.length ? `<g transform="${transforms.join(" ")}">${icon.body}</g>` : icon.body;
  return { body, viewBox: `${fmt(left)} ${fmt(top)} ${fmt(width)} ${fmt(height)}`, width, height };
}

export function parts(icon: IconData, opts: BuildOptions = {}): SvgParts {
  const body = recolor(rewriteStroke(icon.body, opts.strokeWidth), opts.color);
  return applyTransform({ ...icon, body }, combine(icon, { rotate: opts.rotate, hFlip: opts.hFlip, vFlip: opts.vFlip }));
}

export function dimensions(p: Pick<SvgParts, "width" | "height">, size: number): { width: number; height: number } {
  if (p.width === p.height || p.width <= 0 || p.height <= 0) return { width: size, height: size };
  if (p.width > p.height) return { width: size, height: Math.round((size * p.height) / p.width * 100) / 100 };
  return { width: Math.round((size * p.width) / p.height * 100) / 100, height: size };
}

export function buildSvg(icon: IconData, opts: BuildOptions = {}): string {
  const p = parts(icon, opts);
  const { width, height } = dimensions(p, opts.size ?? 24);
  return `<svg xmlns="${SVG_NS}" width="${fmt(width)}" height="${fmt(height)}" viewBox="${p.viewBox}">${p.body}</svg>`;
}

export function previewSvg(icon: IconData, opts: BuildOptions = {}): string {
  const p = parts(icon, opts);
  return `<svg xmlns="${SVG_NS}" width="100%" height="100%" viewBox="${p.viewBox}" aria-hidden="true" focusable="false">${p.body}</svg>`;
}

export function encodeSvgForCss(svg: string): string {
  return svg
    .replace(/"/g, "'")
    .replace(/%/g, "%25")
    .replace(/#/g, "%23")
    .replace(/</g, "%3C")
    .replace(/>/g, "%3E")
    .replace(/\{/g, "%7B")
    .replace(/\}/g, "%7D")
    .replace(/\s+/g, " ");
}

export function dataUri(svg: string): string {
  return `data:image/svg+xml,${encodeSvgForCss(svg)}`;
}

export function cssBackground(icon: IconData, opts: BuildOptions = {}): string {
  const color = safeColor(opts.color) ?? "#000";
  const svg = buildSvg(icon, { ...opts, color });
  const { width, height } = dimensions(parts(icon, opts), opts.size ?? 24);
  return [
    `width: ${fmt(width)}px;`,
    `height: ${fmt(height)}px;`,
    `background: url("${dataUri(svg)}") no-repeat center / contain;`,
  ].join("\n");
}

export function symbolId(prefix: string, name: string): string {
  return `${prefix}-${name}`.replace(/[^a-zA-Z0-9_-]/g, "-");
}

export type SpriteEntry = { prefix: string; icon: IconData };

export function buildSprite(entries: SpriteEntry[], opts: BuildOptions = {}): string {
  const seen = new Set<string>();
  const symbols: string[] = [];
  for (const { prefix, icon } of entries) {
    const id = symbolId(prefix, icon.name);
    if (seen.has(id)) continue;
    seen.add(id);
    const p = parts(icon, opts);
    symbols.push(`  <symbol id="${id}" viewBox="${p.viewBox}">${p.body}</symbol>`);
  }
  return `<svg xmlns="${SVG_NS}" style="display: none">\n${symbols.join("\n")}\n</svg>\n`;
}
