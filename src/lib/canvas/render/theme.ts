import { parseColor, prefersDark, type Rgba } from './color';

export interface CanvasTheme {
  dark: boolean;
  ink: Rgba;
  accent: Rgba;
  surface: Rgba;
  bg: Rgba;
  grid: Rgba;
  cord: Rgba;
}

function token(el: Element, name: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim();
}

export function readTheme(el: Element = document.documentElement): CanvasTheme {
  const dark = prefersDark();
  const ink = parseColor(token(el, '--text') || (dark ? '#f5f5f7' : '#1d1d1f'));
  const accent = parseColor(token(el, '--accent') || '#ff9f0a');
  const surface = parseColor(token(el, '--surface') || (dark ? '#1c1c1e' : '#ffffff'));
  const bg = parseColor(token(el, '--bg') || (dark ? '#111113' : '#f5f5f7'));
  return {
    dark,
    ink,
    accent,
    surface,
    bg,
    grid: [ink[0], ink[1], ink[2], dark ? 0.16 : 0.2],
    cord: ink,
  };
}

export function watchTheme(fn: () => void): () => void {
  if (typeof window === 'undefined') return () => {};
  const mo = new MutationObserver(fn);
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-theme', 'style'] });
  const mq = window.matchMedia?.('(prefers-color-scheme: dark)');
  mq?.addEventListener?.('change', fn);
  return () => {
    mo.disconnect();
    mq?.removeEventListener?.('change', fn);
  };
}
