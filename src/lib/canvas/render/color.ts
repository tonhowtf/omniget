// Colour helpers shared by the GL passes and the Canvas 2D fallback.

export type Rgba = [number, number, number, number];

const cache = new Map<string, Rgba>();

/** Parses #rgb, #rrggbb, #rrggbbaa, rgb()/rgba() and named colours (via Canvas 2D). */
export function parseColor(css: string | undefined, fallback: Rgba = [0.9, 0.9, 0.9, 1]): Rgba {
  if (!css) return fallback;
  const hit = cache.get(css);
  if (hit) return hit;
  let out: Rgba | null = null;
  const s = css.trim();
  if (s.startsWith('#')) {
    const h = s.slice(1);
    const full = h.length === 3 || h.length === 4 ? h.split('').map((c) => c + c).join('') : h;
    if (/^[0-9a-f]{6}([0-9a-f]{2})?$/i.test(full)) {
      const n = parseInt(full.slice(0, 6), 16);
      const a = full.length === 8 ? parseInt(full.slice(6, 8), 16) / 255 : 1;
      out = [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, a];
    }
  } else {
    const m = s.match(/^rgba?\(([^)]+)\)$/i);
    if (m) {
      const parts = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
      if (parts.length >= 3 && parts.every((p) => Number.isFinite(p))) {
        out = [parts[0] / 255, parts[1] / 255, parts[2] / 255, parts[3] ?? 1];
      }
    } else if (typeof document !== 'undefined') {
      const ctx = document.createElement('canvas').getContext('2d');
      if (ctx) {
        ctx.fillStyle = '#000';
        ctx.fillStyle = s;
        const norm = String(ctx.fillStyle);
        if (norm !== s) out = parseColor(norm, fallback);
      }
    }
  }
  const res = out ?? fallback;
  cache.set(css, res);
  return res;
}

export function hexToRgba(hex: number, alpha = 1): Rgba {
  return [((hex >> 16) & 255) / 255, ((hex >> 8) & 255) / 255, (hex & 255) / 255, alpha];
}

export function rgbaCss(c: Rgba): string {
  return `rgba(${Math.round(c[0] * 255)}, ${Math.round(c[1] * 255)}, ${Math.round(c[2] * 255)}, ${c[3]})`;
}

export function withAlpha(c: Rgba, a: number): Rgba {
  return [c[0], c[1], c[2], c[3] * a];
}

/** Reads a CSS custom property from the document root (theme tokens). */
export function themeColor(name: string, fallback: string): string {
  if (typeof document === 'undefined') return fallback;
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/** True when the app background is dark (measured on the --bg token, not the OS setting). */
export function prefersDark(): boolean {
  if (typeof document === 'undefined') return true;
  const bg = getComputedStyle(document.documentElement).getPropertyValue('--bg').trim();
  if (bg) {
    const [r, g, b] = parseColor(bg, [0, 0, 0, 1]);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5;
  }
  return typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches;
}
