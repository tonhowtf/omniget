import { UNICODE_RANGES, cdnUrl, nearestWeight, previewSubsets, variableUrl, type PreviewFont } from "./preview";

const pending = new Map<string, Promise<boolean>>();
const variableFailed = new Set<string>();

function addFace(family: string, url: string, descriptors: FontFaceDescriptors): Promise<boolean> {
  if (typeof FontFace === "undefined" || typeof document === "undefined") return Promise.resolve(false);
  const face = new FontFace(family, `url(${JSON.stringify(url)}) format("woff2")`, descriptors);
  return face
    .load()
    .then((loaded) => {
      document.fonts.add(loaded);
      return true;
    })
    .catch(() => false);
}

function once(key: string, run: () => Promise<boolean>): Promise<boolean> {
  const hit = pending.get(key);
  if (hit) return hit;
  const p = run().then((ok) => {
    if (!ok) pending.delete(key);
    return ok;
  });
  pending.set(key, p);
  return p;
}

export function forgetFailures(): void {
  variableFailed.clear();
}

function descriptors(subset: string, weight: string, style: string): FontFaceDescriptors {
  const d: FontFaceDescriptors = { weight, style, display: "swap" };
  const range = UNICODE_RANGES[subset];
  if (range) d.unicodeRange = range;
  return d;
}

async function loadSubset(font: PreviewFont, subset: string, weight: number, style: string): Promise<boolean> {
  const id = font.id!;
  if (font.variable && font.wghtRange && !variableFailed.has(`${id}:${style}`)) {
    const [lo, hi] = font.wghtRange;
    const ok = await once(`${id}:vf:${subset}:${style}`, () => addFace(font.cssFamily, variableUrl(id, subset, style), descriptors(subset, `${lo} ${hi}`, style)));
    if (ok) return true;
    if (subset === font.defaultSubset) variableFailed.add(`${id}:${style}`);
  }
  const w = nearestWeight(font.weights, weight);
  return once(`${id}:${subset}:${w}:${style}`, () => addFace(font.cssFamily, cdnUrl(id, subset, w, style), descriptors(subset, String(w), style)));
}

export async function ensureFont(font: PreviewFont | null, weight: number, italic: boolean, lang: string): Promise<boolean> {
  if (!font || font.source !== "catalog" || !font.id) return true;
  const style = italic && font.italic ? "italic" : "normal";
  const subsets = previewSubsets({ subsets: font.subsets, default_subset: font.defaultSubset }, lang);
  const results = await Promise.all(subsets.map((s) => loadSubset(font, s, weight, style)));
  return results[0] ?? false;
}
