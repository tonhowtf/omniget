import { NAMED_COLORS } from "./named";

export type Rgb = { r: number; g: number; b: number };
export type Oklab = { l: number; a: number; b: number };
export type Oklch = { l: number; c: number; h: number; alpha?: number };
export type Gamut = "srgb" | "p3";

const EPS = 1e-6;
const GAMUT_EPS = 1e-4;

export function srgbToLinear(v: number): number {
  const a = Math.abs(v);
  const out = a <= 0.04045 ? a / 12.92 : Math.pow((a + 0.055) / 1.055, 2.4);
  return Math.sign(v) * out;
}

export function linearToSrgb(v: number): number {
  const a = Math.abs(v);
  const out = a <= 0.0031308 ? a * 12.92 : 1.055 * Math.pow(a, 1 / 2.4) - 0.055;
  return Math.sign(v) * out;
}

export function linearRgbToOklab({ r, g, b }: Rgb): Oklab {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return {
    l: 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    a: 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    b: 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  };
}

export function oklabToLinearRgb({ l: L, a, b }: Oklab): Rgb {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return {
    r: 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    g: -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    b: -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  };
}

export function oklabToOklch({ l, a, b }: Oklab): Oklch {
  const c = Math.sqrt(a * a + b * b);
  const h = c < EPS ? 0 : normalizeHue((Math.atan2(b, a) * 180) / Math.PI);
  return { l, c, h };
}

export function oklchToOklab({ l, c, h }: Oklch): Oklab {
  const rad = (h * Math.PI) / 180;
  return { l, a: c * Math.cos(rad), b: c * Math.sin(rad) };
}

export function normalizeHue(h: number): number {
  const out = h % 360;
  return out < 0 ? out + 360 : out;
}

export function srgbToOklch(rgb: Rgb, alpha = 1): Oklch {
  const lin = { r: srgbToLinear(rgb.r), g: srgbToLinear(rgb.g), b: srgbToLinear(rgb.b) };
  return { ...oklabToOklch(linearRgbToOklab(lin)), alpha };
}

export function oklchToLinearRgb(color: Oklch): Rgb {
  return oklabToLinearRgb(oklchToOklab(color));
}

export function oklchToSrgb(color: Oklch): Rgb {
  const lin = oklchToLinearRgb(color);
  return { r: linearToSrgb(lin.r), g: linearToSrgb(lin.g), b: linearToSrgb(lin.b) };
}

export function linearSrgbToLinearP3({ r, g, b }: Rgb): Rgb {
  return {
    r: 0.8224621209 * r + 0.1775378791 * g,
    g: 0.0331941989 * r + 0.9668058011 * g,
    b: 0.0170826307 * r + 0.0723974407 * g + 0.9105199286 * b,
  };
}

export function linearP3ToLinearSrgb({ r, g, b }: Rgb): Rgb {
  return {
    r: 1.2249401763 * r - 0.2249401763 * g,
    g: -0.0420569547 * r + 1.0420569547 * g,
    b: -0.0196375546 * r - 0.0786360722 * g + 1.0982736268 * b,
  };
}

export function oklchToP3(color: Oklch): Rgb {
  const p3 = linearSrgbToLinearP3(oklchToLinearRgb(color));
  return { r: linearToSrgb(p3.r), g: linearToSrgb(p3.g), b: linearToSrgb(p3.b) };
}

function within(rgb: Rgb): boolean {
  return [rgb.r, rgb.g, rgb.b].every((v) => v >= -GAMUT_EPS && v <= 1 + GAMUT_EPS);
}

export function inSrgb(color: Oklch): boolean {
  return within(oklchToLinearRgb(color));
}

export function inP3(color: Oklch): boolean {
  return within(linearSrgbToLinearP3(oklchToLinearRgb(color)));
}

export function inGamut(color: Oklch, gamut: Gamut): boolean {
  return gamut === "p3" ? inP3(color) : inSrgb(color);
}

export function fitToGamut(color: Oklch, gamut: Gamut = "srgb"): Oklch {
  const l = clamp(color.l, 0, 1);
  const base = { ...color, l };
  if (l >= 1 - EPS) return { ...base, l: 1, c: 0 };
  if (l <= EPS) return { ...base, l: 0, c: 0 };
  if (inGamut(base, gamut)) return base;
  let lo = 0;
  let hi = color.c;
  for (let i = 0; i < 32; i++) {
    const mid = (lo + hi) / 2;
    if (inGamut({ ...base, c: mid }, gamut)) lo = mid;
    else hi = mid;
  }
  return { ...base, c: lo };
}

export function maxChroma(l: number, h: number, gamut: Gamut = "srgb"): number {
  return fitToGamut({ l, c: 0.5, h }, gamut).c;
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function to255(v: number): number {
  return Math.round(clamp(v, 0, 1) * 255);
}

function hex2(n: number): string {
  return n.toString(16).padStart(2, "0");
}

export function toRgb255(color: Oklch): [number, number, number] {
  const rgb = oklchToSrgb(fitToGamut(color, "srgb"));
  return [to255(rgb.r), to255(rgb.g), to255(rgb.b)];
}

export function toHex(color: Oklch): string {
  const [r, g, b] = toRgb255(color);
  const alpha = color.alpha ?? 1;
  const tail = alpha < 1 ? hex2(Math.round(clamp(alpha, 0, 1) * 255)) : "";
  return `#${hex2(r)}${hex2(g)}${hex2(b)}${tail}`;
}

function trim(n: number, digits: number): string {
  const fixed = n.toFixed(digits);
  const out = fixed.includes(".") ? fixed.replace(/0+$/, "").replace(/\.$/, "") : fixed;
  return out === "-0" ? "0" : out;
}

function alphaTail(alpha: number | undefined): string {
  const a = alpha ?? 1;
  return a < 1 ? ` / ${trim(clamp(a, 0, 1) * 100, 1)}%` : "";
}

export function formatOklch(color: Oklch): string {
  const c = color.c < 0.00005 ? 0 : color.c;
  return `oklch(${trim(color.l * 100, 2)}% ${trim(c, 4)} ${trim(c === 0 ? 0 : color.h, 2)}${alphaTail(color.alpha)})`;
}

export function formatOklab(color: Oklch): string {
  const lab = oklchToOklab(color);
  return `oklab(${trim(lab.l * 100, 2)}% ${trim(lab.a, 4)} ${trim(lab.b, 4)}${alphaTail(color.alpha)})`;
}

export function formatRgb(color: Oklch): string {
  const [r, g, b] = toRgb255(color);
  return `rgb(${r} ${g} ${b}${alphaTail(color.alpha)})`;
}

export function rgbToHsl({ r, g, b }: Rgb): { h: number; s: number; l: number } {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d < EPS) return { h: 0, s: 0, l };
  const s = d / (1 - Math.abs(2 * l - 1));
  let h: number;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return { h: normalizeHue(h * 60), s, l };
}

export function hslToRgb(h: number, s: number, l: number): Rgb {
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1));
  return { r: f(0), g: f(8), b: f(4) };
}

export function formatHsl(color: Oklch): string {
  const [r, g, b] = toRgb255(color);
  const hsl = rgbToHsl({ r: r / 255, g: g / 255, b: b / 255 });
  return `hsl(${trim(hsl.h, 1)} ${trim(hsl.s * 100, 1)}% ${trim(hsl.l * 100, 1)}%${alphaTail(color.alpha)})`;
}

export function formatP3(color: Oklch): string {
  const p3 = oklchToP3(fitToGamut(color, "p3"));
  const ch = (v: number) => trim(clamp(v, 0, 1), 4);
  return `color(display-p3 ${ch(p3.r)} ${ch(p3.g)} ${ch(p3.b)}${alphaTail(color.alpha)})`;
}

export type FormatId = "hex" | "rgb" | "hsl" | "oklch" | "oklab" | "p3";

export const FORMAT_IDS: FormatId[] = ["hex", "rgb", "hsl", "oklch", "oklab", "p3"];

export function formatAs(color: Oklch, id: FormatId): string {
  switch (id) {
    case "hex":
      return toHex(color);
    case "rgb":
      return formatRgb(color);
    case "hsl":
      return formatHsl(color);
    case "oklch":
      return formatOklch(color);
    case "oklab":
      return formatOklab(color);
    case "p3":
      return formatP3(color);
  }
}

function parseHex(input: string): Oklch | null {
  const m = /^#?([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(input);
  if (!m) return null;
  let hex = m[1];
  if (hex.length <= 4) hex = [...hex].map((ch) => ch + ch).join("");
  const n = (i: number) => parseInt(hex.slice(i, i + 2), 16) / 255;
  const alpha = hex.length === 8 ? n(6) : 1;
  return srgbToOklch({ r: n(0), g: n(2), b: n(4) }, alpha);
}

function splitArgs(body: string, legacyAlpha: boolean): { parts: string[]; alpha: string | null } | null {
  const slash = body.split("/");
  if (slash.length > 2) return null;
  const main = slash[0].trim();
  let parts = main.includes(",") ? main.split(",").map((p) => p.trim()) : main.split(/\s+/);
  parts = parts.filter((p) => p.length > 0);
  let alpha = slash.length === 2 ? slash[1].trim() : null;
  if (legacyAlpha && alpha === null && parts.length === 4) alpha = parts.pop() ?? null;
  return { parts, alpha };
}

function num(token: string, percentScale = 1): number | null {
  const t = token.trim().toLowerCase();
  if (t === "none") return 0;
  if (t.endsWith("%")) {
    const v = Number(t.slice(0, -1));
    return Number.isFinite(v) ? (v / 100) * percentScale : null;
  }
  const v = Number(t);
  return Number.isFinite(v) && t.length > 0 ? v : null;
}

function angle(token: string): number | null {
  const t = token.trim().toLowerCase();
  const units: [string, number][] = [["deg", 1], ["grad", 0.9], ["rad", 180 / Math.PI], ["turn", 360]];
  for (const [unit, factor] of units) {
    if (t.endsWith(unit)) {
      const v = Number(t.slice(0, -unit.length));
      return Number.isFinite(v) ? v * factor : null;
    }
  }
  return num(t);
}

function parseAlpha(token: string | null): number | null {
  if (token === null) return 1;
  const v = num(token, 1);
  return v === null ? null : clamp(v, 0, 1);
}

function parseFunction(name: string, body: string): Oklch | null {
  const args = splitArgs(body, name !== "color");
  if (!args) return null;
  const alpha = parseAlpha(args.alpha);
  if (alpha === null) return null;
  const p = args.parts;
  if (name === "color") {
    const space = p[0]?.toLowerCase();
    if (p.length !== 4 || (space !== "srgb" && space !== "display-p3" && space !== "srgb-linear")) return null;
    const v = p.slice(1).map((x) => num(x, 1));
    if (v.some((x) => x === null)) return null;
    const [r, g, b] = v as number[];
    if (space === "srgb") return srgbToOklch({ r, g, b }, alpha);
    if (space === "srgb-linear") return { ...oklabToOklch(linearRgbToOklab({ r, g, b })), alpha };
    const lin = linearP3ToLinearSrgb({ r: srgbToLinear(r), g: srgbToLinear(g), b: srgbToLinear(b) });
    return { ...oklabToOklch(linearRgbToOklab(lin)), alpha };
  }
  if (p.length !== 3) return null;
  if (name === "rgb" || name === "rgba") {
    const v = p.map((x) => {
      const t = x.trim();
      return t.endsWith("%") ? num(t, 1) : (num(t) ?? NaN) / 255;
    });
    if (v.some((x) => x === null || !Number.isFinite(x))) return null;
    const [r, g, b] = v as number[];
    return srgbToOklch({ r: clamp(r, 0, 1), g: clamp(g, 0, 1), b: clamp(b, 0, 1) }, alpha);
  }
  if (name === "hsl" || name === "hsla") {
    const h = angle(p[0]);
    const s = num(p[1].endsWith("%") ? p[1] : `${p[1]}%`, 1);
    const l = num(p[2].endsWith("%") ? p[2] : `${p[2]}%`, 1);
    if (h === null || s === null || l === null) return null;
    return srgbToOklch(hslToRgb(normalizeHue(h), clamp(s, 0, 1), clamp(l, 0, 1)), alpha);
  }
  if (name === "oklch") {
    const l = num(p[0], 1);
    const c = num(p[1], 0.4);
    const h = angle(p[2]);
    if (l === null || c === null || h === null) return null;
    return { l: clamp(l, 0, 1), c: Math.max(0, c), h: normalizeHue(h), alpha };
  }
  if (name === "oklab") {
    const l = num(p[0], 1);
    const a = num(p[1], 0.4);
    const b = num(p[2], 0.4);
    if (l === null || a === null || b === null) return null;
    return { ...oklabToOklch({ l: clamp(l, 0, 1), a, b }), alpha };
  }
  return null;
}

export function parseColor(input: string): Oklch | null {
  const text = input.trim().toLowerCase();
  if (!text) return null;
  if (text === "transparent") return { l: 0, c: 0, h: 0, alpha: 0 };
  const named = NAMED_COLORS[text];
  if (named) return parseHex(named);
  if (text.startsWith("#") || /^[0-9a-f]{6}$/.test(text) || /^[0-9a-f]{3}$/.test(text)) return parseHex(text);
  const m = /^([a-z-]+)\((.*)\)$/.exec(text);
  if (!m) return null;
  return parseFunction(m[1], m[2]);
}

export function deltaEOk(a: Oklch, b: Oklch): number {
  const x = oklchToOklab(a);
  const y = oklchToOklab(b);
  return Math.hypot(x.l - y.l, x.a - y.a, x.b - y.b);
}

export function cssColor(color: Oklch): string {
  return inSrgb(color) ? toHex(color) : formatOklch(color);
}

export function sameColor(a: Oklch, b: Oklch): boolean {
  return deltaEOk(a, b) < 0.0005 && Math.abs((a.alpha ?? 1) - (b.alpha ?? 1)) < 0.002;
}
