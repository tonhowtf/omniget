import {
  fitToGamut,
  formatOklch,
  hslToRgb,
  linearP3ToLinearSrgb,
  linearRgbToOklab,
  normalizeHue,
  oklabToOklch,
  parseColor,
  srgbToLinear,
  srgbToOklch,
  toHex,
  type Oklch,
} from "./color";
import { newId, readProject, scaleOf, slugify, uniqueToken, type PaletteDef, type PaletteNamer, type Project } from "./project";

export type ExportFormat = "css" | "tailwind" | "dtcg" | "scss" | "json";

export const EXPORT_FORMATS: ExportFormat[] = ["css", "tailwind", "dtcg", "scss", "json"];

export const EXTENSIONS: Record<ExportFormat, string> = {
  css: "css",
  tailwind: "css",
  dtcg: "tokens.json",
  scss: "scss",
  json: "palette.json",
};

export const EXTENSION_KEY = "wtf.tonho.omniget.palette";

type Row = { token: string; step: number; color: Oklch };

function rows(project: Project): Row[] {
  const out: Row[] = [];
  for (const palette of project.palettes) {
    for (const s of scaleOf(palette, project.gamut)) out.push({ token: palette.token, step: s.step, color: s.color });
  }
  return out;
}

function oklchOut(project: Project, color: Oklch): string {
  return formatOklch(fitToGamut(color, project.gamut));
}

export function exportCss(project: Project): string {
  const list = rows(project);
  const hex = list.map((r) => `  --${r.token}-${r.step}: ${toHex(r.color)};`).join("\n");
  const ok = list.map((r) => `    --${r.token}-${r.step}: ${oklchOut(project, r.color)};`).join("\n");
  return `:root {\n${hex}\n}\n\n@supports (color: oklch(0% 0 0)) {\n  :root {\n${ok}\n  }\n}\n`;
}

export function exportTailwind(project: Project): string {
  const list = rows(project);
  const body = list.map((r) => `  --color-${r.token}-${r.step}: ${oklchOut(project, r.color)};`).join("\n");
  return `@import "tailwindcss";\n\n@theme {\n${body}\n}\n`;
}

export function exportScss(project: Project): string {
  const lines: string[] = [];
  for (const palette of project.palettes) {
    const scale = scaleOf(palette, project.gamut);
    for (const s of scale) lines.push(`$${palette.token}-${s.step}: ${toHex(s.color)};`);
    lines.push(`$${palette.token}: (`);
    scale.forEach((s, i) => lines.push(`  ${s.step}: $${palette.token}-${s.step}${i < scale.length - 1 ? "," : ""}`));
    lines.push(");", "");
  }
  return lines.join("\n");
}

export function exportDtcg(project: Project): string {
  const color: Record<string, unknown> = {};
  for (const palette of project.palettes) {
    const group: Record<string, unknown> = {};
    for (const s of scaleOf(palette, project.gamut)) {
      group[String(s.step)] = {
        $type: "color",
        $value: toHex(s.color),
        $extensions: { [EXTENSION_KEY]: { oklch: oklchOut(project, s.color), base: s.isBase } },
      };
    }
    group.$description = palette.name;
    color[palette.token] = group;
  }
  return `${JSON.stringify({ color }, null, 2)}\n`;
}

export function exportJson(project: Project): string {
  const { id: _id, updatedAt: _u, ...rest } = project;
  return `${JSON.stringify({ format: EXTENSION_KEY, version: 1, project: rest }, null, 2)}\n`;
}

export function exportAs(project: Project, format: ExportFormat): string {
  switch (format) {
    case "css":
      return exportCss(project);
    case "tailwind":
      return exportTailwind(project);
    case "dtcg":
      return exportDtcg(project);
    case "scss":
      return exportScss(project);
    case "json":
      return exportJson(project);
  }
}

export function fileName(project: Project, format: ExportFormat): string {
  const stem = slugify(project.name, "palette");
  return format === "tailwind" ? `${stem}.theme.css` : `${stem}.${EXTENSIONS[format]}`;
}

export type ImportResult = { ok: true; project: Project; kind: "project" | "dtcg" } | { ok: false; reason: "json" | "shape" | "empty" };

function tokenValue(node: unknown): Oklch | null {
  if (!node || typeof node !== "object") return null;
  const o = node as Record<string, unknown>;
  const ext = (o.$extensions as Record<string, Record<string, unknown>> | undefined)?.[EXTENSION_KEY];
  if (ext && typeof ext.oklch === "string") {
    const c = parseColor(ext.oklch);
    if (c) return c;
  }
  const v = o.$value ?? o.value;
  if (typeof v === "string") return parseColor(v);
  if (v && typeof v === "object") {
    const obj = v as Record<string, unknown>;
    if (typeof obj.hex === "string") return parseColor(obj.hex);
    if (typeof obj.colorSpace === "string" && Array.isArray(obj.components)) {
      return fromComponents(obj.colorSpace, obj.components, obj.alpha);
    }
  }
  return null;
}

function component(v: unknown): number | null {
  if (v === "none") return 0;
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function fromComponents(space: string, raw: unknown[], alphaRaw: unknown): Oklch | null {
  if (raw.length < 3) return null;
  const parts = raw.slice(0, 3).map(component);
  if (parts.some((p) => p === null)) return null;
  const [a, b, c] = parts as number[];
  const alpha = typeof alphaRaw === "number" && Number.isFinite(alphaRaw) ? Math.min(1, Math.max(0, alphaRaw)) : 1;
  switch (space) {
    case "srgb":
      return srgbToOklch({ r: a, g: b, b: c }, alpha);
    case "srgb-linear":
      return { ...oklabToOklch(linearRgbToOklab({ r: a, g: b, b: c })), alpha };
    case "display-p3": {
      const linear = linearP3ToLinearSrgb({ r: srgbToLinear(a), g: srgbToLinear(b), b: srgbToLinear(c) });
      return { ...oklabToOklch(linearRgbToOklab(linear)), alpha };
    }
    case "hsl":
      return srgbToOklch(hslToRgb(normalizeHue(a), b / 100, c / 100), alpha);
    case "oklab":
      return { ...oklabToOklch({ l: a, a: b, b: c }), alpha };
    case "oklch":
      return { l: a, c: b, h: normalizeHue(c), alpha };
    default:
      return null;
  }
}

function paletteFromGroup(name: string, group: Record<string, unknown>, taken: string[]): PaletteDef | null {
  const steps: { step: number; color: Oklch; base: boolean }[] = [];
  for (const [key, node] of Object.entries(group)) {
    if (key.startsWith("$")) continue;
    const step = Number(key);
    const color = tokenValue(node);
    if (!color || !Number.isFinite(step)) continue;
    const ext = ((node as Record<string, unknown>).$extensions as Record<string, Record<string, unknown>> | undefined)?.[EXTENSION_KEY];
    steps.push({ step, color, base: ext?.base === true });
  }
  if (steps.length === 0) return null;
  steps.sort((a, b) => a.step - b.step);
  const marked = steps.find((s) => s.base);
  const byStep = steps.find((s) => s.step === 500);
  const pick = marked ?? byStep ?? steps[Math.floor(steps.length / 2)];
  const label = typeof group.$description === "string" && group.$description.trim() ? group.$description.trim() : name;
  const token = uniqueToken(slugify(name, `palette-${taken.length + 1}`), taken);
  return { id: newId("pal"), name: label, token, base: { ...pick.color, alpha: 1 }, hueShift: 0 };
}

export function importText(text: string, fallbackName: string, untitled: PaletteNamer): ImportResult {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return { ok: false, reason: "json" };
  }
  if (!data || typeof data !== "object") return { ok: false, reason: "shape" };
  const o = data as Record<string, unknown>;
  const wrapped = o.format === EXTENSION_KEY ? o.project : Array.isArray(o.palettes) ? o : null;
  if (wrapped) {
    const project = readProject(wrapped, fallbackName, untitled);
    if (!project) return { ok: false, reason: "shape" };
    if (project.palettes.length === 0) return { ok: false, reason: "empty" };
    return { ok: true, project: { ...project, id: newId("prj"), updatedAt: Date.now() }, kind: "project" };
  }
  const root = (o.color && typeof o.color === "object" ? o.color : o) as Record<string, unknown>;
  const palettes: PaletteDef[] = [];
  for (const [name, group] of Object.entries(root)) {
    if (name.startsWith("$") || !group || typeof group !== "object") continue;
    const pal = paletteFromGroup(name, group as Record<string, unknown>, palettes.map((p) => p.token));
    if (pal) palettes.push(pal);
  }
  if (palettes.length === 0) return { ok: false, reason: "empty" };
  const project = readProject({ name: fallbackName, palettes }, fallbackName, untitled);
  if (!project) return { ok: false, reason: "shape" };
  return { ok: true, project, kind: "dtcg" };
}
