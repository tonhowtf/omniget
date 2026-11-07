export type FlatToken = { path: string; type: string | null; value: string };

export type FlattenResult = { tokens: FlatToken[]; unresolved: string[]; duplicates: string[] };

export const SHADCN_VARS = [
  "background",
  "foreground",
  "card",
  "card-foreground",
  "popover",
  "popover-foreground",
  "primary",
  "primary-foreground",
  "secondary",
  "secondary-foreground",
  "muted",
  "muted-foreground",
  "accent",
  "accent-foreground",
  "destructive",
  "destructive-foreground",
  "border",
  "input",
  "ring",
  "radius",
  "chart-1",
  "chart-2",
  "chart-3",
  "chart-4",
  "chart-5",
  "sidebar",
  "sidebar-foreground",
  "sidebar-primary",
  "sidebar-primary-foreground",
  "sidebar-accent",
  "sidebar-accent-foreground",
  "sidebar-border",
  "sidebar-ring",
] as const;

export type Mode = "light" | "dark";

export type MappingRow = { name: string; light: string | null; dark: string | null };

const MAX_DEPTH = 32;
const MAX_TOKENS = 5_000;

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function tokenOf(node: Record<string, unknown>): { value: unknown; type: unknown } | null {
  if ("$value" in node) return { value: node.$value, type: node.$type };
  if ("value" in node && "type" in node && typeof node.type === "string" && !isObject(node.value)) {
    return { value: node.value, type: node.type };
  }
  if ("value" in node && "type" in node && isObject(node.value) && !Object.values(node.value).some(isObject)) {
    return { value: node.value, type: node.type };
  }
  return null;
}

function num(v: unknown): string | null {
  return typeof v === "number" && Number.isFinite(v) ? String(Math.round(v * 10_000) / 10_000) : null;
}

function hex2(n: number): string {
  return Math.round(Math.max(0, Math.min(1, n)) * 255)
    .toString(16)
    .padStart(2, "0");
}

export function cssValue(raw: unknown, type: string | null): string | null {
  if (typeof raw === "string") {
    const s = raw.trim();
    if (!s || /[;{}<>]/.test(s) || s.length > 300) return null;
    return s;
  }
  if (typeof raw === "number") return num(raw);
  if (Array.isArray(raw) && type === "fontFamily") {
    const names = raw.filter((x): x is string => typeof x === "string" && !/[;{}<>"]/.test(x));
    return names.length ? names.map((n) => (/\s/.test(n) ? `"${n}"` : n)).join(", ") : null;
  }
  if (isObject(raw)) {
    if (typeof raw.hex === "string" && /^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(raw.hex)) {
      const hex = raw.hex.toLowerCase();
      if (hex.length === 5 || hex.length === 9) return hex;
      const full = hex.length === 4 ? `#${[...hex.slice(1)].map((c) => c + c).join("")}` : hex;
      const alpha = typeof raw.alpha === "number" && raw.alpha < 1 ? hex2(raw.alpha) : "";
      return `${full}${alpha}`;
    }
    if (Array.isArray(raw.components) && typeof raw.colorSpace === "string") {
      const comps = raw.components.map((c) => (c === "none" ? "none" : num(c)));
      if (comps.length !== 3 || comps.some((c) => c === null)) return null;
      const alpha = typeof raw.alpha === "number" && raw.alpha < 1 ? ` / ${num(raw.alpha)}` : "";
      const space = raw.colorSpace.toLowerCase();
      const hasNone = comps.includes("none");
      if (space === "srgb" && !hasNone) {
        const [r, g, b] = raw.components as number[];
        return alpha ? `rgb(${[r, g, b].map((c) => Math.round(c * 255)).join(" ")}${alpha})` : `#${hex2(r)}${hex2(g)}${hex2(b)}`;
      }
      if (space === "oklch" || space === "oklab" || space === "lab" || space === "lch") return `${space}(${comps.join(" ")}${alpha})`;
      if (space === "hsl") return `hsl(${comps[0]} ${comps[1] === "none" ? "none" : `${comps[1]}%`} ${comps[2] === "none" ? "none" : `${comps[2]}%`}${alpha})`;
      if (/^[a-z0-9-]+$/.test(space)) return `color(${space} ${comps.join(" ")}${alpha})`;
      return null;
    }
    if (typeof raw.value === "number" && typeof raw.unit === "string" && /^[a-z%]{1,5}$/.test(raw.unit)) {
      return `${num(raw.value)}${raw.unit}`;
    }
  }
  return null;
}

function setOrder(doc: unknown): string[] {
  if (!isObject(doc)) return [];
  const groups = Object.keys(doc).filter((k) => !k.startsWith("$") && isObject(doc[k]) && !tokenOf(doc[k] as Record<string, unknown>));
  const meta = isObject(doc.$metadata) ? doc.$metadata.tokenSetOrder : undefined;
  const listed = Array.isArray(meta) ? meta.filter((k): k is string => typeof k === "string" && groups.includes(k)) : [];
  return [...listed, ...groups.filter((k) => !listed.includes(k))];
}

export function flattenDtcg(doc: unknown): FlattenResult {
  const raw: { path: string; type: string | null; value: unknown }[] = [];
  const walk = (node: unknown, path: string[], inherited: string | null, depth: number) => {
    if (!isObject(node) || depth > MAX_DEPTH || raw.length >= MAX_TOKENS) return;
    const groupType = typeof node.$type === "string" ? node.$type : inherited;
    const token = tokenOf(node);
    if (token && path.length > 0) {
      raw.push({ path: path.join("."), type: typeof token.type === "string" ? token.type : groupType, value: token.value });
      return;
    }
    for (const [key, child] of Object.entries(node)) {
      if (key.startsWith("$")) continue;
      walk(child, [...path, key], groupType, depth + 1);
    }
  };
  walk(doc, [], null, 0);

  const byPath = new Map<string, (typeof raw)[number]>();
  const duplicates = new Set<string>();
  for (const t of raw) {
    if (byPath.has(t.path)) duplicates.add(t.path);
    else byPath.set(t.path, t);
  }
  const sets = setOrder(doc);
  const lookup = (ref: string, from: string): (typeof raw)[number] | undefined => {
    const direct = byPath.get(ref);
    if (direct) return direct;
    const own = from.split(".")[0];
    const prefixes = sets.includes(own) ? [own, ...sets.filter((x) => x !== own)] : sets;
    for (const set of prefixes) {
      const hit = byPath.get(`${set}.${ref}`);
      if (hit) return hit;
    }
    return undefined;
  };
  const unresolved = new Set<string>();
  const resolve = (value: unknown, from: string, seen: Set<string>): unknown => {
    if (typeof value !== "string") return value;
    const whole = /^\{([^{}]+)\}$/.exec(value.trim());
    if (whole) {
      const target = lookup(whole[1], from);
      if (!target || seen.has(target.path)) return undefined;
      return resolve(target.value, target.path, new Set([...seen, target.path]));
    }
    if (/\{[^{}]+\}/.test(value)) {
      let failed = false;
      const out = value.replace(/\{([^{}]+)\}/g, (_, ref: string) => {
        const target = lookup(ref, from);
        const v = target && !seen.has(target.path) ? resolve(target.value, target.path, new Set([...seen, target.path])) : undefined;
        const css = v === undefined ? null : cssValue(v, target?.type ?? null);
        if (css === null) failed = true;
        return css ?? "";
      });
      return failed ? undefined : out;
    }
    return value;
  };

  const tokens: FlatToken[] = [];
  for (const t of byPath.values()) {
    const resolved = resolve(t.value, t.path, new Set([t.path]));
    const css = resolved === undefined ? null : cssValue(resolved, t.type);
    if (css === null) unresolved.add(t.path);
    else tokens.push({ path: t.path, type: t.type, value: css });
  }
  return { tokens, unresolved: [...unresolved], duplicates: [...duplicates] };
}

const MODE_WORDS = new Set(["light", "dark", "day", "night"]);
const NOISE = new Set(["color", "colors", "semantic", "theme", "themes", "sys", "system", "global", "base", "default", "shadcn", "tokens", "ref", "alias", "value"]);

const ALIASES: Record<string, string[]> = {
  background: ["bg", "surface-default", "canvas"],
  foreground: ["fg", "text", "text-default", "on-background", "on-surface"],
  "card-foreground": ["on-card", "card-text"],
  "popover-foreground": ["on-popover", "popover-text"],
  "primary-foreground": ["on-primary", "primary-fg", "primary-text", "primary-contrast"],
  "secondary-foreground": ["on-secondary", "secondary-fg", "secondary-text"],
  "muted-foreground": ["on-muted", "muted-fg", "muted-text", "text-muted", "text-subtle"],
  "accent-foreground": ["on-accent", "accent-fg", "accent-text"],
  destructive: ["danger", "error", "critical"],
  "destructive-foreground": ["on-danger", "on-error", "on-destructive", "danger-foreground"],
  border: ["border-default", "stroke", "outline"],
  input: ["input-border", "border-input", "field-border"],
  ring: ["focus", "focus-ring", "ring-focus"],
  radius: ["radius-md", "border-radius", "radius-default", "corner-radius"],
};

export function tokenMode(path: string): Mode | null {
  const segs = path.toLowerCase().split(/[.\s/]+/);
  if (segs.some((s) => s === "dark" || s === "night")) return "dark";
  if (segs.some((s) => s === "light" || s === "day")) return "light";
  return null;
}

export function tokenKey(path: string): string {
  return path
    .replace(/([a-z])([A-Z])/g, "$1-$2")
    .toLowerCase()
    .split(/[.\s/_]+/)
    .filter((s) => s && !MODE_WORDS.has(s) && !NOISE.has(s))
    .join("-");
}

function matches(key: string, name: string): number {
  if (key === name) return 3;
  if ((ALIASES[name] ?? []).includes(key)) return 2;
  if (key.endsWith(`-${name}`) && !SHADCN_VARS.some((v) => v !== name && v.endsWith(`-${name}`) && key.endsWith(v))) return 1;
  return 0;
}

export function suggestMapping(tokens: FlatToken[]): MappingRow[] {
  return SHADCN_VARS.map((name) => {
    const pick = (mode: Mode) => {
      let best: { path: string; score: number } | null = null;
      for (const t of tokens) {
        const m = tokenMode(t.path);
        if (mode === "dark" ? m !== "dark" : m === "dark") continue;
        const s = matches(tokenKey(t.path), name);
        if (s > 0 && (!best || s > best.score)) best = { path: t.path, score: s };
      }
      return best?.path ?? null;
    };
    return { name, light: pick("light"), dark: pick("dark") };
  });
}

type Rgba = { r: number; g: number; b: number; a: number };

function parseAlpha(raw: string | undefined): number | null {
  if (raw === undefined) return 1;
  const v = raw.trim();
  if (v === "none") return 0;
  const n = v.endsWith("%") ? parseFloat(v) / 100 : parseFloat(v);
  return Number.isFinite(n) ? Math.max(0, Math.min(1, n)) : null;
}

function splitArgs(body: string): { parts: string[]; alpha: string | undefined } | null {
  const [main, alpha, extra] = body.split("/");
  if (extra !== undefined) return null;
  const parts = main.trim().split(/[\s,]+/).filter(Boolean);
  if (alpha === undefined && parts.length === 4) return { parts: parts.slice(0, 3), alpha: parts[3] };
  return parts.length === 3 ? { parts, alpha } : null;
}

function numberOf(raw: string, percentScale: number): number | null {
  if (raw === "none") return 0;
  const n = parseFloat(raw);
  if (!Number.isFinite(n) || !/^-?(\d+\.?\d*|\.\d+)(e-?\d+)?(%|deg)?$/i.test(raw)) return null;
  return raw.endsWith("%") ? (n / 100) * percentScale : n;
}

function toGamma(c: number): number {
  const v = Math.abs(c) <= 0.0031308 ? 12.92 * c : Math.sign(c) * (1.055 * Math.pow(Math.abs(c), 1 / 2.4) - 0.055);
  return Math.max(0, Math.min(1, v));
}

function oklabToRgb(L: number, A: number, B: number): { r: number; g: number; b: number } {
  const l = Math.pow(L + 0.3963377774 * A + 0.2158037573 * B, 3);
  const m = Math.pow(L - 0.1055613458 * A - 0.0638541728 * B, 3);
  const s = Math.pow(L - 0.0894841775 * A - 1.291485548 * B, 3);
  return {
    r: toGamma(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    g: toGamma(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    b: toGamma(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  };
}

function hslToRgb(h: number, s: number, l: number): { r: number; g: number; b: number } {
  const hue = (((h % 360) + 360) % 360) / 60;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs((hue % 2) - 1));
  const [r, g, b] = hue < 1 ? [c, x, 0] : hue < 2 ? [x, c, 0] : hue < 3 ? [0, c, x] : hue < 4 ? [0, x, c] : hue < 5 ? [x, 0, c] : [c, 0, x];
  const m = l - c / 2;
  return { r: r + m, g: g + m, b: b + m };
}

export function parseColor(value: string): Rgba | null {
  const v = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(v);
  if (hex) {
    const h = hex[1].length <= 4 ? [...hex[1]].map((c) => c + c).join("") : hex[1];
    const [r, g, b, a] = [0, 2, 4, 6].map((i) => (i < h.length ? parseInt(h.slice(i, i + 2), 16) / 255 : 1));
    return { r, g, b, a };
  }
  const fn = /^(rgba?|hsla?|oklch|oklab|color)\(\s*([^()]*)\)$/.exec(v);
  if (!fn) return null;
  let body = fn[2];
  const name = fn[1];
  if (name === "color") {
    const sp = /^(srgb)\s+(.*)$/.exec(body);
    if (!sp) return null;
    body = sp[2];
  }
  const args = splitArgs(body);
  if (!args) return null;
  const a = parseAlpha(args.alpha);
  if (a === null) return null;
  const [p0, p1, p2] = args.parts;
  if (name === "color") {
    const c = [p0, p1, p2].map((x) => numberOf(x, 1));
    if (c.some((x) => x === null)) return null;
    const [r, g, b] = c as number[];
    return { r, g, b, a };
  }
  if (name.startsWith("rgb")) {
    const c = [p0, p1, p2].map((x) => numberOf(x, 255));
    if (c.some((x) => x === null)) return null;
    const [r, g, b] = (c as number[]).map((x) => Math.max(0, Math.min(1, x / 255)));
    return { r, g, b, a };
  }
  if (name.startsWith("hsl")) {
    const h = numberOf(p0, 360);
    const sat = numberOf(p1.endsWith("%") || p1 === "none" ? p1 : `${p1}%`, 1);
    const l = numberOf(p2.endsWith("%") || p2 === "none" ? p2 : `${p2}%`, 1);
    if (h === null || sat === null || l === null) return null;
    return { ...hslToRgb(h, Math.max(0, Math.min(1, sat)), Math.max(0, Math.min(1, l))), a };
  }
  const L = numberOf(p0, 1);
  if (L === null) return null;
  if (name === "oklab") {
    const A = numberOf(p1, 0.4);
    const B = numberOf(p2, 0.4);
    if (A === null || B === null) return null;
    return { ...oklabToRgb(L, A, B), a };
  }
  const C = numberOf(p1, 0.4);
  const H = numberOf(p2, 360);
  if (C === null || H === null) return null;
  const rad = (H * Math.PI) / 180;
  return { ...oklabToRgb(L, C * Math.cos(rad), C * Math.sin(rad)), a };
}

export function toHslChannels(value: string): string | null {
  if (HSL_CHANNELS.test(value.trim())) return value.trim();
  const c = parseColor(value);
  if (!c) return null;
  const { r, g, b } = c;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  let hue = 0;
  let sat = 0;
  if (max - min > 1e-9) {
    const d = max - min;
    sat = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) hue = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) hue = (b - r) / d + 2;
    else hue = (r - g) / d + 4;
    hue *= 60;
  }
  const r1 = (n: number) => Math.round(n * 10) / 10;
  const alpha = c.a < 1 ? ` / ${Math.round(c.a * 1000) / 1000}` : "";
  return `${r1(hue)} ${r1(sat * 100)}% ${r1(l * 100)}%${alpha}`;
}

const HSL_CHANNELS = /^-?\d+(\.\d+)?(deg)?\s+\d+(\.\d+)?%\s+\d+(\.\d+)?%(\s*\/\s*\d*\.?\d+%?)?$/;

export function adaptValue(value: string, existing: string | null): string | null {
  if (existing && HSL_CHANNELS.test(existing.trim())) return toHslChannels(value);
  return value;
}

function findBlock(css: string, selector: string): { open: number; close: number } | null {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(`(^|[\\s,;{}])${esc}\\s*\\{`, "g");
  let m: RegExpExecArray | null;
  while ((m = re.exec(css))) {
    const open = m.index + m[0].length - 1;
    let depth = 0;
    for (let i = open; i < css.length; i++) {
      const c = css[i];
      if (c === "{") depth++;
      else if (c === "}") {
        depth--;
        if (depth === 0) {
          const body = css.slice(open + 1, i);
          if (!body.includes("{")) return { open, close: i };
          break;
        }
      }
    }
  }
  return null;
}

export type VarChange = { name: string; before: string | null; after: string };

export type ApplyResult = { css: string; changes: VarChange[]; skipped: string[] };

export function applyVars(css: string, selector: string, vars: Record<string, string>): ApplyResult {
  const entries = Object.entries(vars).filter(([, v]) => v);
  const skipped: string[] = [];
  if (entries.length === 0) return { css, changes: [], skipped };
  const changes: VarChange[] = [];
  const block = findBlock(css, selector);
  if (!block) {
    const lines = entries.map(([n, v]) => {
      changes.push({ name: n, before: null, after: v });
      return `  --${n}: ${v};`;
    });
    const sep = css.length === 0 || css.endsWith("\n\n") ? "" : css.endsWith("\n") ? "\n" : "\n\n";
    return { css: `${css}${sep}${selector} {\n${lines.join("\n")}\n}\n`, changes, skipped };
  }
  let body = css.slice(block.open + 1, block.close);
  const indent = /\n([ \t]+)--/.exec(body)?.[1] ?? "  ";
  const sample = [...body.matchAll(/--[a-z0-9-]+\s*:\s*([^;]*);/g)].map((m) => m[1].trim()).find((v) => HSL_CHANNELS.test(v)) ?? null;
  const missing: string[] = [];
  for (const [name, value] of entries) {
    const re = new RegExp(`(--${name.replace(/[-]/g, "\\-")}\\s*:\\s*)([^;]*)(;?)`);
    const m = re.exec(body);
    if (m) {
      const before = m[2].trim();
      const after = name === "radius" ? value : adaptValue(value, before);
      if (after === null) {
        skipped.push(name);
        continue;
      }
      if (before !== after) {
        changes.push({ name, before, after });
        body = body.slice(0, m.index) + m[1] + after + ";" + body.slice(m.index + m[0].length);
      }
    } else {
      const after = name === "radius" ? value : adaptValue(value, sample);
      if (after === null) {
        skipped.push(name);
        continue;
      }
      changes.push({ name, before: null, after });
      missing.push(`${indent}--${name}: ${after};`);
    }
  }
  if (missing.length) {
    const trimmed = body.replace(/\s*$/, "");
    const closeIndent = /\n([ \t]*)$/.exec(body)?.[1] ?? "";
    body = `${trimmed}\n${missing.join("\n")}\n${closeIndent}`;
  }
  return { css: css.slice(0, block.open + 1) + body + css.slice(block.close), changes, skipped };
}

export function readVar(css: string, selector: string, name: string): string | null {
  const block = findBlock(css, selector);
  if (!block) return null;
  const m = new RegExp(`--${name.replace(/[-]/g, "\\-")}\\s*:\\s*([^;]*);`).exec(css.slice(block.open + 1, block.close));
  return m ? m[1].trim() : null;
}

export function buildVars(rows: MappingRow[], tokens: FlatToken[], mode: Mode): Record<string, string> {
  const byPath = new Map(tokens.map((t) => [t.path, t.value]));
  const out: Record<string, string> = {};
  for (const row of rows) {
    const path = mode === "light" ? row.light : row.dark;
    const value = path ? byPath.get(path) : undefined;
    if (value) out[row.name] = value;
  }
  return out;
}

export function cssSnippet(light: Record<string, string>, dark: Record<string, string>): string {
  const block = (sel: string, vars: Record<string, string>) =>
    Object.keys(vars).length ? `${sel} {\n${Object.entries(vars).map(([n, v]) => `  --${n}: ${v};`).join("\n")}\n}\n` : "";
  return [block(":root", light), block(".dark", dark)].filter(Boolean).join("\n");
}

export type DiffLine = { kind: "same" | "add" | "del"; text: string; a: number | null; b: number | null };

export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const out: DiffLine[] = [];
  for (let i = 0; i < start; i++) out.push({ kind: "same", text: a[i], a: i + 1, b: i + 1 });
  const ma = a.slice(start, endA);
  const mb = b.slice(start, endB);
  if (ma.length * mb.length > 4_000_000) {
    ma.forEach((text, i) => out.push({ kind: "del", text, a: start + i + 1, b: null }));
    mb.forEach((text, i) => out.push({ kind: "add", text, a: null, b: start + i + 1 }));
  } else {
    const lcs: number[][] = Array.from({ length: ma.length + 1 }, () => new Array(mb.length + 1).fill(0));
    for (let i = ma.length - 1; i >= 0; i--)
      for (let j = mb.length - 1; j >= 0; j--) lcs[i][j] = ma[i] === mb[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    let i = 0;
    let j = 0;
    while (i < ma.length || j < mb.length) {
      if (i < ma.length && j < mb.length && ma[i] === mb[j]) {
        out.push({ kind: "same", text: ma[i], a: start + i + 1, b: start + j + 1 });
        i++;
        j++;
      } else if (i < ma.length && (j >= mb.length || lcs[i + 1][j] >= lcs[i][j + 1])) {
        out.push({ kind: "del", text: ma[i], a: start + i + 1, b: null });
        i++;
      } else {
        out.push({ kind: "add", text: mb[j], a: null, b: start + j + 1 });
        j++;
      }
    }
  }
  for (let k = 0; k < a.length - endA; k++) out.push({ kind: "same", text: a[endA + k], a: endA + k + 1, b: endB + k + 1 });
  return out;
}

export type DiffHunk = { lines: DiffLine[] };

export function hunks(diff: DiffLine[], context = 2): DiffHunk[] {
  const keep = new Array(diff.length).fill(false);
  diff.forEach((d, i) => {
    if (d.kind === "same") return;
    for (let k = Math.max(0, i - context); k <= Math.min(diff.length - 1, i + context); k++) keep[k] = true;
  });
  const out: DiffHunk[] = [];
  let cur: DiffLine[] | null = null;
  diff.forEach((d, i) => {
    if (keep[i]) {
      if (!cur) {
        cur = [];
        out.push({ lines: cur });
      }
      cur.push(d);
    } else cur = null;
  });
  return out;
}
