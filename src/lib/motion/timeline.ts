import { bezierEase, clampBezier, type Bezier } from "./bezier";
import { cssBezier } from "./exports";
import { num } from "./format";

export type TrackProp = "x" | "y" | "scale" | "rotate" | "opacity" | "color";

export const TRACK_PROPS: TrackProp[] = ["x", "y", "scale", "rotate", "opacity", "color"];

export type Keyframe = { id: string; time: number; value: number | string; ease: Bezier };

export type StageKind = "box" | "text" | "image";

export type Stage = { kind: StageKind; text: string; imageName: string | null };

export type Timeline = {
  version: 1;
  name: string;
  duration: number;
  stage: Stage;
  tracks: Record<TrackProp, Keyframe[]>;
};

export type Frame = { x: number; y: number; scale: number; rotate: number; opacity: number; color: string };

export const DEFAULTS: Frame = { x: 0, y: 0, scale: 1, rotate: 0, opacity: 1, color: "#ff9f0a" };

export const LIMITS: Record<Exclude<TrackProp, "color">, { min: number; max: number; step: number; unit: string }> = {
  x: { min: -400, max: 400, step: 1, unit: "px" },
  y: { min: -200, max: 200, step: 1, unit: "px" },
  scale: { min: 0, max: 4, step: 0.01, unit: "" },
  rotate: { min: -720, max: 720, step: 1, unit: "deg" },
  opacity: { min: 0, max: 1, step: 0.01, unit: "" },
};

export const MIN_DURATION = 100;
export const MAX_DURATION = 60000;

let seq = 0;
export function keyId(): string {
  seq += 1;
  return `k${Date.now().toString(36)}${seq.toString(36)}`;
}

export function emptyTracks(): Record<TrackProp, Keyframe[]> {
  return { x: [], y: [], scale: [], rotate: [], opacity: [], color: [] };
}

export function sampleTimeline(): Timeline {
  const out: Bezier = [0.16, 1, 0.3, 1];
  const inOut: Bezier = [0.42, 0, 0.58, 1];
  const back: Bezier = [0.34, 1.56, 0.64, 1];
  const k = (time: number, value: number | string, ease: Bezier): Keyframe => ({ id: keyId(), time, value, ease });
  return {
    version: 1,
    name: "",
    duration: 2000,
    stage: { kind: "box", text: "Motion", imageName: null },
    tracks: {
      x: [k(0, -160, out), k(900, 160, inOut), k(2000, -160, inOut)],
      y: [],
      scale: [k(0, 0.6, back), k(600, 1, inOut)],
      rotate: [k(0, 0, inOut), k(900, 180, inOut), k(2000, 360, inOut)],
      opacity: [k(0, 0, out), k(400, 1, out)],
      color: [k(0, "#ff9f0a", inOut), k(1000, "#5edbc8", inOut), k(2000, "#ff9f0a", inOut)],
    },
  };
}

function parseHex(hex: string): [number, number, number] {
  let h = hex.trim().replace(/^#/, "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = Number.parseInt(h.slice(0, 6), 16);
  if (!Number.isFinite(n) || h.length < 6) return [0, 0, 0];
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function toHex([r, g, b]: [number, number, number]): string {
  const c = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}

export function isHex(value: string): boolean {
  return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value.trim());
}

export function mixColor(a: string, b: string, f: number): string {
  const ca = parseHex(a);
  const cb = parseHex(b);
  return toHex([ca[0] + (cb[0] - ca[0]) * f, ca[1] + (cb[1] - ca[1]) * f, ca[2] + (cb[2] - ca[2]) * f]);
}

export function sorted(track: Keyframe[]): Keyframe[] {
  return track.slice().sort((a, b) => a.time - b.time);
}

export function valueAt(prop: TrackProp, track: Keyframe[], time: number): number | string {
  const keys = sorted(track);
  if (keys.length === 0) return DEFAULTS[prop];
  if (time <= keys[0].time) return keys[0].value;
  const last = keys[keys.length - 1];
  if (time >= last.time) return last.value;
  for (let i = 1; i < keys.length; i++) {
    const b = keys[i];
    if (time <= b.time) {
      const a = keys[i - 1];
      const span = b.time - a.time;
      const f = span <= 0 ? 1 : bezierEase(clampBezier(a.ease))((time - a.time) / span);
      if (prop === "color") return mixColor(String(a.value), String(b.value), f);
      return Number(a.value) + (Number(b.value) - Number(a.value)) * f;
    }
  }
  return last.value;
}

export function frameAt(tl: Timeline, time: number): Frame {
  return {
    x: Number(valueAt("x", tl.tracks.x, time)),
    y: Number(valueAt("y", tl.tracks.y, time)),
    scale: Number(valueAt("scale", tl.tracks.scale, time)),
    rotate: Number(valueAt("rotate", tl.tracks.rotate, time)),
    opacity: Number(valueAt("opacity", tl.tracks.opacity, time)),
    color: String(valueAt("color", tl.tracks.color, time)),
  };
}

export function colorProperty(stage: Stage): "background-color" | "color" {
  return stage.kind === "text" ? "color" : "background-color";
}

function cssValue(prop: TrackProp, value: number | string): string {
  switch (prop) {
    case "x":
    case "y":
      return `${num(Number(value), 2)}px`;
    case "rotate":
      return `${num(Number(value), 2)}deg`;
    case "color":
      return String(value);
    default:
      return num(Number(value), 3);
  }
}

function cssProperty(prop: TrackProp, stage: Stage): string {
  if (prop === "x") return "--motion-x";
  if (prop === "y") return "--motion-y";
  if (prop === "color") return colorProperty(stage);
  return prop;
}

export function activeTracks(tl: Timeline): TrackProp[] {
  return TRACK_PROPS.filter((p) => tl.tracks[p].length > 0);
}

export function exportTracks(tl: Timeline): TrackProp[] {
  return activeTracks(tl).filter((p) => p !== "color" || tl.stage.kind !== "image");
}

export type CssOptions = { name: string; iterations: number | "infinite"; fill: boolean };

export function safeName(name: string): string {
  const cleaned = name.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  return /^[a-z_]/.test(cleaned) ? cleaned : `motion-${cleaned || "timeline"}`;
}

export function toCssKeyframes(tl: Timeline, opts: CssOptions): string {
  const base = safeName(opts.name);
  const props = exportTracks(tl);
  const duration = Math.max(1, tl.duration);
  const blocks: string[] = [];
  if (props.includes("x") || props.includes("y")) {
    blocks.push("@property --motion-x { syntax: '<length>'; inherits: false; initial-value: 0px; }");
    blocks.push("@property --motion-y { syntax: '<length>'; inherits: false; initial-value: 0px; }");
  }
  const names: string[] = [];
  for (const prop of props) {
    const keys = sorted(tl.tracks[prop]);
    const name = `${base}-${prop}`;
    names.push(name);
    const property = cssProperty(prop, tl.stage);
    const lines = [`@keyframes ${name} {`];
    if (keys[0].time > 0) lines.push(`  0% { ${property}: ${cssValue(prop, keys[0].value)}; }`);
    keys.forEach((key, i) => {
      const offset = num((Math.min(duration, Math.max(0, key.time)) / duration) * 100, 3);
      const ease = i < keys.length - 1 ? ` animation-timing-function: ${cssBezier(clampBezier(key.ease))};` : "";
      lines.push(`  ${offset}% { ${property}: ${cssValue(prop, key.value)};${ease} }`);
    });
    const last = keys[keys.length - 1];
    if (last.time < duration) lines.push(`  100% { ${property}: ${cssValue(prop, last.value)}; }`);
    lines.push("}");
    blocks.push(lines.join("\n"));
  }
  const iterations = opts.iterations === "infinite" ? "infinite" : String(Math.max(1, Math.round(opts.iterations)));
  const fill = opts.fill ? " both" : "";
  const shorthand = names.map((n) => `${n} ${Math.round(duration)}ms linear ${iterations}${fill}`).join(",\n    ");
  const decl = [`.${base} {`];
  if (props.includes("x") || props.includes("y")) decl.push("  translate: var(--motion-x) var(--motion-y);");
  decl.push(names.length ? `  animation:\n    ${shorthand};` : "  animation: none;");
  decl.push("}");
  blocks.push(decl.join("\n"));
  return blocks.join("\n\n");
}

export type WaapiOptions = { iterations: number | "infinite"; fill: boolean; selector: string };

function waapiProperty(prop: TrackProp, stage: Stage): string {
  if (prop === "x") return "--motion-x";
  if (prop === "y") return "--motion-y";
  if (prop === "color") return stage.kind === "text" ? "color" : "backgroundColor";
  return prop;
}

export function toWaapi(tl: Timeline, opts: WaapiOptions): string {
  const props = exportTracks(tl);
  const duration = Math.max(1, Math.round(tl.duration));
  const iterations = opts.iterations === "infinite" ? "Infinity" : String(Math.max(1, Math.round(opts.iterations)));
  const lines = [`const el = document.querySelector(${JSON.stringify(opts.selector)});`];
  if (props.includes("x") || props.includes("y")) {
    lines.push("for (const name of [\"--motion-x\", \"--motion-y\"]) {");
    lines.push("  try { CSS.registerProperty({ name, syntax: \"<length>\", inherits: false, initialValue: \"0px\" }); } catch {}");
    lines.push("}");
    lines.push("el.style.translate = \"var(--motion-x) var(--motion-y)\";");
  }
  lines.push(`const timing = { duration: ${duration}, iterations: ${iterations}${opts.fill ? ", fill: \"both\"" : ""} };`);
  lines.push("const animations = [");
  for (const prop of props) {
    const keys = sorted(tl.tracks[prop]);
    const key = waapiProperty(prop, tl.stage);
    const field = key.startsWith("--") ? JSON.stringify(key) : key;
    const frames: string[] = [];
    if (keys[0].time > 0) frames.push(`{ offset: 0, ${field}: ${JSON.stringify(cssValue(prop, keys[0].value))} }`);
    keys.forEach((k, i) => {
      const offset = num(Math.min(1, Math.max(0, k.time / duration)), 4);
      const ease = i < keys.length - 1 ? `, easing: ${JSON.stringify(cssBezier(clampBezier(k.ease)))}` : "";
      frames.push(`{ offset: ${offset}, ${field}: ${JSON.stringify(cssValue(prop, k.value))}${ease} }`);
    });
    const last = keys[keys.length - 1];
    if (last.time < duration) frames.push(`{ offset: 1, ${field}: ${JSON.stringify(cssValue(prop, last.value))} }`);
    lines.push(`  el.animate([\n    ${frames.join(",\n    ")},\n  ], timing),`);
  }
  lines.push("];");
  return lines.join("\n");
}

function isBezier(v: unknown): v is Bezier {
  return Array.isArray(v) && v.length === 4 && v.every((n) => typeof n === "number" && Number.isFinite(n));
}

export type ParseError = "not_object" | "bad_version" | "bad_tracks";

export type ParseResult = { ok: true; timeline: Timeline; imageDropped: boolean } | { ok: false; error: ParseError };

export function parseTimeline(text: string): ParseResult {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return { ok: false, error: "not_object" };
  }
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return { ok: false, error: "not_object" };
  const obj = raw as Record<string, unknown>;
  if (obj.version !== 1) return { ok: false, error: "bad_version" };
  const tracksRaw = obj.tracks;
  if (!tracksRaw || typeof tracksRaw !== "object") return { ok: false, error: "bad_tracks" };
  const duration = Math.min(MAX_DURATION, Math.max(MIN_DURATION, Number(obj.duration) || 2000));
  const tracks = emptyTracks();
  for (const prop of TRACK_PROPS) {
    const list = (tracksRaw as Record<string, unknown>)[prop];
    if (list === undefined) continue;
    if (!Array.isArray(list)) return { ok: false, error: "bad_tracks" };
    for (const item of list) {
      if (!item || typeof item !== "object") return { ok: false, error: "bad_tracks" };
      const k = item as Record<string, unknown>;
      const time = Number(k.time);
      if (!Number.isFinite(time)) return { ok: false, error: "bad_tracks" };
      let value: number | string;
      if (prop === "color") {
        if (typeof k.value !== "string" || !isHex(k.value)) return { ok: false, error: "bad_tracks" };
        value = k.value;
      } else {
        value = Number(k.value);
        if (!Number.isFinite(value)) return { ok: false, error: "bad_tracks" };
      }
      tracks[prop].push({
        id: keyId(),
        time: Math.min(duration, Math.max(0, time)),
        value,
        ease: isBezier(k.ease) ? clampBezier(k.ease) : [0.42, 0, 0.58, 1],
      });
    }
    tracks[prop] = sorted(tracks[prop]);
  }
  const stageRaw = (obj.stage ?? {}) as Record<string, unknown>;
  const kind: StageKind = stageRaw.kind === "text" || stageRaw.kind === "image" ? stageRaw.kind : "box";
  const stage: Stage = {
    kind: kind === "image" ? "box" : kind,
    text: typeof stageRaw.text === "string" ? stageRaw.text.slice(0, 80) : "Motion",
    imageName: null,
  };
  const name = typeof obj.name === "string" ? obj.name.trim().slice(0, 80) : "";
  return { ok: true, timeline: { version: 1, name, duration, stage, tracks }, imageDropped: kind === "image" };
}

export function serializeTimeline(tl: Timeline): string {
  const tracks: Record<string, { time: number; value: number | string; ease: Bezier }[]> = {};
  for (const prop of TRACK_PROPS) {
    tracks[prop] = sorted(tl.tracks[prop]).map((k) => ({ time: Math.round(k.time), value: k.value, ease: k.ease }));
  }
  const stage = { kind: tl.stage.kind, text: tl.stage.text };
  return JSON.stringify({ version: 1, name: tl.name, duration: Math.round(tl.duration), stage, tracks }, null, 2);
}

export function keyCount(tl: Timeline): number {
  return TRACK_PROPS.reduce((n, p) => n + tl.tracks[p].length, 0);
}
