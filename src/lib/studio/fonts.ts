import { BUNDLED_FONT } from "./presets";

const FILES: [string, string][] = [
  ["400", "/vendor/pdfjs/standard_fonts/LiberationSans-Regular.ttf"],
  ["700", "/vendor/pdfjs/standard_fonts/LiberationSans-Bold.ttf"],
];

let loading: Promise<void> | null = null;

export function loadBundledFont(): Promise<void> {
  if (loading) return loading;
  if (typeof document === "undefined" || typeof FontFace === "undefined") return Promise.resolve();
  loading = Promise.all(
    FILES.map(async ([weight, url]) => {
      const face = new FontFace(BUNDLED_FONT, `url(${url})`, { weight });
      await face.load();
      document.fonts.add(face);
    }),
  )
    .then(() => undefined)
    .catch(() => undefined);
  return loading;
}

export function cssFontStack(font: string): string {
  const safe = font.replace(/["\\]/g, "");
  return `"${safe}", "${BUNDLED_FONT}", sans-serif`;
}

const TOKENS = ["--accent", "--blue", "--purple", "--teal", "--success", "--warning", "--error", "--primary", "--secondary"];

function toHex(color: string): string | null {
  const probe = document.createElement("canvas").getContext("2d");
  if (!probe) return null;
  probe.fillStyle = "#000000";
  probe.fillStyle = color;
  const v = probe.fillStyle;
  return /^#[0-9a-f]{6}$/i.test(v) ? v.toUpperCase() : null;
}

export function tokenSwatches(): string[] {
  if (typeof document === "undefined") return [];
  const css = getComputedStyle(document.documentElement);
  const out = new Set<string>(["#FFFFFF", "#000000"]);
  for (const token of TOKENS) {
    const raw = css.getPropertyValue(token).trim();
    const hex = raw ? toHex(raw) : null;
    if (hex) out.add(hex);
  }
  return [...out];
}
