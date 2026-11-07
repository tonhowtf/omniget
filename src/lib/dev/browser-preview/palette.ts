type Args = Record<string, unknown>;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let stored: unknown = null;

const SUNSET = [
  { hex: "#1d2a4a", l: 0.2853, c: 0.0623, h: 266.1, share: 0.27 },
  { hex: "#f29e4c", l: 0.7616, c: 0.1316, h: 62.4, share: 0.19 },
  { hex: "#c8553d", l: 0.5797, c: 0.1407, h: 33.9, share: 0.15 },
  { hex: "#5b6c8f", l: 0.5277, c: 0.0601, h: 265.4, share: 0.12 },
  { hex: "#f6d7a7", l: 0.8946, c: 0.0624, h: 76.3, share: 0.1 },
  { hex: "#8c3b4a", l: 0.4557, c: 0.1104, h: 9.6, share: 0.08 },
  { hex: "#2f6f73", l: 0.4993, c: 0.0656, h: 199.8, share: 0.05 },
  { hex: "#e9e4dc", l: 0.9205, c: 0.0099, h: 79.4, share: 0.04 },
  { hex: "#3c3c3c", l: 0.3485, c: 0, h: 0, share: 0.0 },
  { hex: "#a05a2c", l: 0.5385, c: 0.1102, h: 51.1, share: 0.0 },
  { hex: "#6e8b3d", l: 0.5918, c: 0.1179, h: 126.2, share: 0.0 },
  { hex: "#d9a0b4", l: 0.7741, c: 0.0674, h: 352.9, share: 0.0 },
];

function previewSvg(): string {
  const bands = SUNSET.slice(0, 8)
    .map((c, i) => `<rect x="0" y="${i * 30}" width="360" height="30" fill="${c.hex}"/>`)
    .join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="360" height="240">${bands}<circle cx="250" cy="110" r="46" fill="#f6d7a7"/></svg>`;
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

function normalized(count: number) {
  const picked = SUNSET.slice(0, count).map((c, i) => ({ ...c, share: c.share || 0.01 * (12 - i) }));
  const total = picked.reduce((sum, c) => sum + c.share, 0);
  return picked.map((c) => ({ ...c, share: Math.round((c.share / total) * 10000) / 10000 }));
}

export const paletteFixtures: Record<string, (args: Args) => unknown> = {
  palette_projects_load: async () => {
    await sleep(240);
    return { data: stored, moved_aside: null };
  },
  palette_projects_save: async ({ data }) => {
    await sleep(80);
    stored = JSON.parse(JSON.stringify(data));
    return null;
  },
  palette_extract: async ({ path, count }) => {
    await sleep(700);
    const p = String(path);
    if (p.includes("broken")) throw "This image could not be read: the file ends before the image data.";
    return { colors: normalized(Math.max(1, Math.min(12, Number(count) || 6))), width: 4032, height: 3024, preview: previewSvg() };
  },
  palette_save_file: async ({ fileName }) => {
    await sleep(160);
    return `/preview/Documents/${String(fileName)}`;
  },
};

export function paletteDialog(_options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/palette")) return undefined;
  return "/preview/Pictures/sunset-over-lisbon.jpg";
}
