import { clamp, normalizeHue, parseColor, type Gamut, type Oklch } from "./color";
import { EASINGS, SPACES, type Bezier, type GradientDef, type Stop } from "./gradient";
import { generateScale, type ScaleStep } from "./scale";

export type PaletteDef = {
  id: string;
  name: string;
  token: string;
  base: Oklch;
  hueShift: number;
};

export type Project = {
  id: string;
  name: string;
  gamut: Gamut;
  palettes: PaletteDef[];
  gradient: GradientDef;
  contrast: { text: Oklch; background: Oklch };
  updatedAt: number;
};

export type ProjectsFile = { version: 1; activeId: string | null; projects: Project[] };

export type Seed = { key: string; token: string; base: Oklch; hueShift: number };

export const DEFAULT_SEEDS: Seed[] = [
  { key: "brand", token: "brand", base: { l: 0.6, c: 0.2, h: 262 }, hueShift: -8 },
  { key: "neutral", token: "neutral", base: { l: 0.6, c: 0.014, h: 262 }, hueShift: 0 },
  { key: "accent", token: "accent", base: { l: 0.68, c: 0.17, h: 48 }, hueShift: 10 },
  { key: "success", token: "success", base: { l: 0.63, c: 0.17, h: 150 }, hueShift: 0 },
  { key: "warning", token: "warning", base: { l: 0.78, c: 0.16, h: 80 }, hueShift: -12 },
  { key: "danger", token: "danger", base: { l: 0.6, c: 0.21, h: 25 }, hueShift: 6 },
];

export function newId(prefix = "p"): string {
  const rnd = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID().slice(0, 8) : Math.random().toString(36).slice(2, 10);
  return `${prefix}-${Date.now().toString(36)}-${rnd}`;
}

export function slugify(name: string, fallback: string): string {
  const slug = name
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (!slug) return fallback;
  return /^[0-9]/.test(slug) ? `c-${slug}` : slug;
}

export function uniqueToken(token: string, taken: string[], selfIndex = -1): string {
  const others = new Set(taken.filter((_, i) => i !== selfIndex));
  if (!others.has(token)) return token;
  let n = 2;
  while (others.has(`${token}-${n}`)) n++;
  return `${token}-${n}`;
}

export function defaultGradient(a: Oklch, b: Oklch): GradientDef {
  return {
    stops: [
      { color: { ...a, alpha: 1 }, pos: 0 },
      { color: { ...b, alpha: 1 }, pos: 1 },
    ],
    space: "oklch",
    easing: "ease-in-out",
    bezier: [0.42, 0, 0.58, 1],
    angle: 90,
    samples: 8,
  };
}

export function makeProject(name: string, nameOf: (key: string) => string): Project {
  const palettes = DEFAULT_SEEDS.map((s) => ({ id: newId("pal"), name: nameOf(s.key), token: s.token, base: { ...s.base, alpha: 1 }, hueShift: s.hueShift }));
  return {
    id: newId("prj"),
    name,
    gamut: "srgb",
    palettes,
    gradient: defaultGradient({ l: 0.56, c: 0.2, h: 262 }, { l: 0.78, c: 0.16, h: 80 }),
    contrast: { text: { l: 0.25, c: 0.02, h: 262, alpha: 1 }, background: { l: 0.97, c: 0.005, h: 262, alpha: 1 } },
    updatedAt: Date.now(),
  };
}

export function scaleOf(palette: PaletteDef, gamut: Gamut): ScaleStep[] {
  return generateScale(palette.base, { hueShift: palette.hueShift, gamut });
}

function finite(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

export function readColor(v: unknown): Oklch | null {
  if (typeof v === "string") return parseColor(v);
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (typeof o.l !== "number" || typeof o.c !== "number") return null;
  return { l: clamp(finite(o.l, 0.5), 0, 1), c: clamp(finite(o.c, 0), 0, 0.5), h: normalizeHue(finite(o.h, 0)), alpha: clamp(finite(o.alpha, 1), 0, 1) };
}

function readStops(v: unknown): Stop[] {
  if (!Array.isArray(v)) return [];
  return v
    .map((s) => {
      const color = readColor((s as Record<string, unknown>)?.color);
      return color ? { color, pos: clamp(finite((s as Record<string, unknown>).pos, 0), 0, 1) } : null;
    })
    .filter((s): s is Stop => s !== null)
    .slice(0, 16);
}

function readGradient(v: unknown, fallback: GradientDef): GradientDef {
  if (!v || typeof v !== "object") return fallback;
  const o = v as Record<string, unknown>;
  const stops = readStops(o.stops);
  const bezier = Array.isArray(o.bezier) && o.bezier.length === 4 && o.bezier.every((n) => typeof n === "number" && Number.isFinite(n)) ? (o.bezier as Bezier) : fallback.bezier;
  return {
    stops: stops.length >= 2 ? stops : fallback.stops,
    space: SPACES.includes(o.space as never) ? (o.space as GradientDef["space"]) : fallback.space,
    easing: EASINGS.includes(o.easing as never) ? (o.easing as GradientDef["easing"]) : fallback.easing,
    bezier: [clamp(bezier[0], 0, 1), bezier[1], clamp(bezier[2], 0, 1), bezier[3]],
    angle: finite(o.angle, fallback.angle),
    samples: clamp(Math.round(finite(o.samples, fallback.samples)), 2, 24),
  };
}

export type PaletteNamer = (n: number) => string;

export function readPalette(v: unknown, index: number, untitled: PaletteNamer): PaletteDef | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  const base = readColor(o.base);
  if (!base) return null;
  const name = typeof o.name === "string" && o.name.trim() ? o.name.trim().slice(0, 60) : untitled(index + 1);
  const token = typeof o.token === "string" && o.token.trim() ? slugify(o.token, `palette-${index + 1}`) : slugify(name, `palette-${index + 1}`);
  return {
    id: typeof o.id === "string" && o.id ? o.id : newId("pal"),
    name,
    token,
    base,
    hueShift: clamp(finite(o.hueShift, 0), -90, 90),
  };
}

export function readProject(v: unknown, fallbackName: string, untitled: PaletteNamer): Project | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!Array.isArray(o.palettes)) return null;
  const palettes: PaletteDef[] = [];
  o.palettes.forEach((p, i) => {
    const pal = readPalette(p, i, untitled);
    if (pal) palettes.push({ ...pal, token: uniqueToken(pal.token, palettes.map((x) => x.token)) });
  });
  const first = palettes[0]?.base ?? { l: 0.56, c: 0.2, h: 262, alpha: 1 };
  const second = palettes[1]?.base ?? { l: 0.78, c: 0.16, h: 80, alpha: 1 };
  const contrast = (o.contrast ?? {}) as Record<string, unknown>;
  return {
    id: typeof o.id === "string" && o.id ? o.id : newId("prj"),
    name: typeof o.name === "string" && o.name.trim() ? o.name.trim().slice(0, 80) : fallbackName,
    gamut: o.gamut === "p3" ? "p3" : "srgb",
    palettes,
    gradient: readGradient(o.gradient, defaultGradient(first, second)),
    contrast: {
      text: readColor(contrast.text) ?? { l: 0.25, c: 0.02, h: first.h, alpha: 1 },
      background: readColor(contrast.background) ?? { l: 0.97, c: 0.005, h: first.h, alpha: 1 },
    },
    updatedAt: finite(o.updatedAt, Date.now()),
  };
}

export function readProjectsFile(v: unknown, fallbackName: string, untitled: PaletteNamer): ProjectsFile {
  const o = (v && typeof v === "object" ? v : {}) as Record<string, unknown>;
  const list = Array.isArray(o.projects) ? o.projects : [];
  const projects = list.map((p) => readProject(p, fallbackName, untitled)).filter((p): p is Project => p !== null);
  const activeId = typeof o.activeId === "string" && projects.some((p) => p.id === o.activeId) ? o.activeId : (projects[0]?.id ?? null);
  return { version: 1, activeId, projects };
}
