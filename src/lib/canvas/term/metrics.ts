import { terminalFont } from '#components/central/terminal/theme.ts';
import { canvasPrefs } from '../prefs.svelte';

export const TERM_FONT_SIZE = 13;
const TERM_LINE_HEIGHT = 1.25;
export const TERM_HEADER_H = 30;
export const TERM_FOOTER_H = 22;
const TERM_PAD = 6;

interface CellMetrics {
  font: string;
  w: number;
  h: number;
  ascent: number;
}

let cached: CellMetrics | null = null;

function fontFamily(): string {
  if (typeof document === 'undefined') return 'ui-monospace, Menlo, Consolas, monospace';
  const own = canvasPrefs().terminal.font_family.trim();
  return own ? `${/[\s,]/.test(own) && !/["',]/.test(own) ? `"${own}"` : own}, ${terminalFont()}` : terminalFont();
}

export function cellMetrics(): CellMetrics {
  if (cached) return cached;
  const font = fontFamily();
  let w = TERM_FONT_SIZE * 0.6;
  let ascent = 0.8;
  try {
    const c =
      typeof OffscreenCanvas !== 'undefined'
        ? new OffscreenCanvas(8, 8)
        : typeof document !== 'undefined'
          ? document.createElement('canvas')
          : null;
    const ctx = c?.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null | undefined;
    if (ctx) {
      ctx.font = `${TERM_FONT_SIZE}px ${font}`;
      const m = ctx.measureText('MMMMMMMMMM');
      if (m.width > 0) w = m.width / 10;
      const asc = m.fontBoundingBoxAscent || m.actualBoundingBoxAscent;
      const desc = m.fontBoundingBoxDescent || m.actualBoundingBoxDescent;
      if (asc > 0 && desc >= 0) {
        const h = TERM_FONT_SIZE * TERM_LINE_HEIGHT;
        ascent = (asc + (h - asc - desc) / 2) / h;
      }
    }
  } catch {
  }
  cached = { font, w, h: TERM_FONT_SIZE * TERM_LINE_HEIGHT, ascent };
  return cached;
}

export function resetMetrics(): void {
  cached = null;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export function bodyRect(node: { x: number; y: number; w: number; h: number }): Rect {
  const p = termPadding();
  return {
    x: node.x + p.x,
    y: node.y + TERM_HEADER_H + p.y,
    w: Math.max(0, node.w - p.x * 2),
    h: Math.max(0, node.h - TERM_HEADER_H - TERM_FOOTER_H - p.y * 2),
  };
}

export function termPadding(): { x: number; y: number } {
  const p = canvasPrefs().terminal.padding;
  return { x: TERM_PAD + Math.max(0, p.x || 0), y: Math.max(0, p.y || 0) };
}

export function termFontScale(node: { data?: unknown } | null | undefined): number {
  const a = (node?.data as { appearance?: { fontSize?: unknown } } | null | undefined)?.appearance;
  const own = typeof a?.fontSize === 'number' && a.fontSize > 0 ? a.fontSize : 0;
  const global = canvasPrefs().terminal.font_size;
  const size = own || (global > 0 ? global : TERM_FONT_SIZE);
  return Math.max(0.5, Math.min(3, size / TERM_FONT_SIZE));
}

export function gridSizeFor(w: number, h: number, fontScale = 1): { cols: number; rows: number } {
  const m = cellMetrics();
  const b = bodyRect({ x: 0, y: 0, w, h });
  return {
    cols: Math.max(2, Math.floor(b.w / (m.w * fontScale))),
    rows: Math.max(1, Math.floor(b.h / (m.h * fontScale))),
  };
}
