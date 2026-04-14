const MASTER_NAMES = new Set([
  "master",
  "playlist",
  "index",
  "main",
  "manifest",
  "stream",
]);

const QUALITY_LABELS = new Set([
  "high",
  "medium",
  "low",
  "sd",
  "hd",
  "fhd",
  "uhd",
  "best",
  "auto",
]);

export function getHlsVariantToken(filename) {
  if (!filename) return "";
  const lower = String(filename).toLowerCase();
  const extStripped = lower.replace(/\.m3u8(\?.*)?$/, "");
  const base = extStripped.split("?")[0];
  if (!base) return "";
  if (MASTER_NAMES.has(base)) return "master";
  if (/^\d{3,4}p?$/.test(base)) return "master";
  if (QUALITY_LABELS.has(base)) return "master";
  return base;
}

export function getHlsGroupKey(url) {
  try {
    const u = new URL(url);
    const parts = u.pathname.split("/");
    const filename = parts.pop() || "";
    const dir = parts.join("/");
    const variant = getHlsVariantToken(filename);
    return `${u.origin}${dir}|${variant}`;
  } catch {
    return url;
  }
}
