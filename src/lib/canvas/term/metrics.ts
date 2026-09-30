// Geometry shared by the GL pass, the input layer and the node chrome: where
// the terminal body sits inside a node and how big one cell is in world units.

import { terminalFont } from '$components/central/terminal/theme';
import { canvasPrefs } from '../prefs.svelte';

/** Base font size of a terminal in world units (CSS px at zoom 1). */
export const TERM_FONT_SIZE = 13;
export const TERM_LINE_HEIGHT = 1.25;
/** Height of the DOM chrome (title bar) on top of the body, world units. */
export const TERM_HEADER_H = 30;
/** Inner padding around the body, world units. */
export const TERM_PAD = 6;

export interface CellMetrics {
  font: string;
  /** Cell size in world units at TERM_FONT_SIZE. */
  w: number;
  h: number;
  /** Baseline offset from the cell top, as a fraction of the cell height. */
  ascent: number;
}

let cached: CellMetrics | null = null;

export function fontFamily(): string {
  if (typeof document === 'undefined') return 'ui-monospace, Menlo, Consolas, monospace';
  // Settings → Canvas → Terminal font (falls back to the app's mono stack).
  const own = canvasPrefs().terminal.font_family.trim();
  return own ? `${/[\s,]/.test(own) && !/["',]/.test(own) ? `"${own}"` : own}, ${terminalFont()}` : terminalFont();
}

/** Measures the monospace advance once (cheap, cached until `resetMetrics`). */
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
    // keep the estimate
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

/** Body rectangle of a terminal node in world units. */
export function bodyRect(node: { x: number; y: number; w: number; h: number }): Rect {
  const p = termPadding();
  return {
    x: node.x + p.x,
    y: node.y + TERM_HEADER_H + p.y,
    w: Math.max(0, node.w - p.x * 2),
    h: Math.max(0, node.h - TERM_HEADER_H - TERM_PAD - p.y * 2),
  };
}

/** Body padding: the base margin plus Settings → Canvas → Terminal padding. */
export function termPadding(): { x: number; y: number } {
  const p = canvasPrefs().terminal.padding;
  return { x: TERM_PAD + Math.max(0, p.x || 0), y: Math.max(0, p.y || 0) };
}

/**
 * Font scale of one terminal: its own size (node `data.appearance.fontSize`,
 * ⌘⇧+/⌘⇧−), else Settings → Canvas → Terminal font size, over the base 13.
 */
export function termFontScale(node: { data?: unknown } | null | undefined): number {
  const a = (node?.data as { appearance?: { fontSize?: unknown } } | null | undefined)?.appearance;
  const own = typeof a?.fontSize === 'number' && a.fontSize > 0 ? a.fontSize : 0;
  const global = canvasPrefs().terminal.font_size;
  const size = own || (global > 0 ? global : TERM_FONT_SIZE);
  return Math.max(0.5, Math.min(3, size / TERM_FONT_SIZE));
}

/** Columns and rows that fit a node of this size (never below 2×1). */
export function gridSizeFor(w: number, h: number, fontScale = 1): { cols: number; rows: number } {
  const m = cellMetrics();
  const b = bodyRect({ x: 0, y: 0, w, h });
  return {
    cols: Math.max(2, Math.floor(b.w / (m.w * fontScale))),
    rows: Math.max(1, Math.floor(b.h / (m.h * fontScale))),
  };
}
