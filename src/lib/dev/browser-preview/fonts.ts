import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

const ALL = [100, 200, 300, 400, 500, 600, 700, 800, 900];

type Fixture = { id: string; family: string; category: string; weights: number[]; italic: boolean; variable: boolean; license?: string; subsets?: string[] };

const CATALOG: Fixture[] = [
  { id: "geist", family: "Geist", category: "sans-serif", weights: ALL, italic: true, variable: true, subsets: ["cyrillic", "latin", "latin-ext", "vietnamese"] },
  { id: "geist-mono", family: "Geist Mono", category: "monospace", weights: ALL, italic: true, variable: true, subsets: ["cyrillic", "latin", "latin-ext"] },
  { id: "inter", family: "Inter", category: "sans-serif", weights: ALL, italic: true, variable: true, subsets: ["cyrillic", "cyrillic-ext", "greek", "latin", "latin-ext", "vietnamese"] },
  { id: "jetbrains-mono", family: "JetBrains Mono", category: "monospace", weights: [100, 200, 300, 400, 500, 600, 700, 800], italic: true, variable: true },
  { id: "ibm-plex-sans", family: "IBM Plex Sans", category: "sans-serif", weights: [100, 200, 300, 400, 500, 600, 700], italic: true, variable: true },
  { id: "ibm-plex-mono", family: "IBM Plex Mono", category: "monospace", weights: [100, 200, 300, 400, 500, 600, 700], italic: true, variable: false },
  { id: "source-sans-3", family: "Source Sans 3", category: "sans-serif", weights: [200, 300, 400, 500, 600, 700, 800, 900], italic: true, variable: true },
  { id: "fira-code", family: "Fira Code", category: "monospace", weights: [300, 400, 500, 600, 700], italic: false, variable: true },
  { id: "atkinson-hyperlegible", family: "Atkinson Hyperlegible", category: "sans-serif", weights: [400, 700], italic: true, variable: false },
  { id: "manrope", family: "Manrope", category: "sans-serif", weights: [200, 300, 400, 500, 600, 700, 800], italic: false, variable: true },
  { id: "space-grotesk", family: "Space Grotesk", category: "sans-serif", weights: [300, 400, 500, 600, 700], italic: false, variable: true },
  { id: "noto-sans", family: "Noto Sans", category: "sans-serif", weights: ALL, italic: true, variable: true },
  { id: "lora", family: "Lora", category: "serif", weights: [400, 500, 600, 700], italic: true, variable: true },
  { id: "fraunces", family: "Fraunces", category: "serif", weights: ALL, italic: true, variable: true },
  { id: "playfair-display", family: "Playfair Display", category: "serif", weights: [400, 500, 600, 700, 800, 900], italic: true, variable: true },
  { id: "dm-serif-display", family: "DM Serif Display", category: "serif", weights: [400], italic: true, variable: false },
  { id: "bricolage-grotesque", family: "Bricolage Grotesque", category: "sans-serif", weights: [200, 300, 400, 500, 600, 700, 800], italic: false, variable: true },
  { id: "bebas-neue", family: "Bebas Neue", category: "display", weights: [400], italic: false, variable: false },
  { id: "caveat", family: "Caveat", category: "handwriting", weights: [400, 500, 600, 700], italic: false, variable: true },
  { id: "ubuntu", family: "Ubuntu", category: "sans-serif", weights: [300, 400, 500, 700], italic: true, variable: false, license: "UFL-1.0" },
  { id: "ubuntu-mono", family: "Ubuntu Mono", category: "monospace", weights: [400, 700], italic: true, variable: false, license: "UFL-1.0" },
  { id: "roboto", family: "Roboto", category: "sans-serif", weights: ALL, italic: true, variable: true, license: "Apache-2.0" },
  { id: "material-symbols-outlined", family: "Material Symbols Outlined", category: "icons", weights: [100, 200, 300, 400, 500, 600, 700], italic: false, variable: true, license: "Apache-2.0" },
];

function catalogFont(f: Fixture) {
  const license = f.license ?? "OFL-1.1";
  const open = /^(ofl|apache|mit)/i.test(license);
  return {
    id: f.id,
    family: f.family,
    category: f.category,
    subsets: f.subsets ?? ["latin", "latin-ext"],
    default_subset: "latin",
    weights: f.weights,
    styles: f.italic ? ["normal", "italic"] : ["normal"],
    variable: f.variable,
    license,
    license_class: open ? "open" : "other",
    source: f.id === "ibm-plex-mono" ? "other" : "google",
  };
}

function details(id: string) {
  const f = CATALOG.find((c) => c.id === id);
  if (!f) throw "Fontsource has no font with this id.";
  const base = catalogFont(f);
  const files = [];
  for (const subset of base.subsets) {
    for (const weight of f.weights) {
      for (const style of base.styles) {
        const url = `https://cdn.jsdelivr.net/fontsource/fonts/${id}@5.2.0/${subset}-${weight}-${style}`;
        files.push({ weight, style, subset, ttf: `${url}.ttf`, woff2: `${url}.woff2` });
      }
    }
  }
  return { ...base, version: "5.2.0", files };
}

type Style = [string, number, boolean];

function family(name: string, styles: Style[], opts: { mono?: boolean; variable?: boolean; user?: boolean; file?: string } = {}) {
  const dir = opts.user ? "/Users/preview/Library/Fonts" : "/System/Library/Fonts";
  const file = opts.file ?? `${dir}/${name.replace(/ /g, "")}.ttc`;
  return {
    family: name,
    styles: styles.map(([n, weight, italic], index) => ({ name: n, weight, italic, path: file, index })),
    weights: [...new Set([...styles.map((s) => s[1]), ...(opts.variable ? [100, 200, 300, 400, 500, 600, 700, 800, 900] : [])])].sort((a, b) => a - b),
    has_italic: styles.some((s) => s[2]),
    is_variable: !!opts.variable,
    axes: opts.variable ? [{ tag: "wght", min: 100, default: 400, max: 900 }] : [],
    is_monospace: !!opts.mono,
    files: [file],
    user_installed: !!opts.user,
  };
}

const REGULAR_BOLD: Style[] = [["Regular", 400, false], ["Italic", 400, true], ["Bold", 700, false], ["Bold Italic", 700, true]];

const INSTALLED = [
  family("American Typewriter", [["Regular", 400, false], ["Bold", 700, false]]),
  family("Avenir Next", [["Regular", 400, false], ["Medium", 500, false], ["Demi Bold", 600, false], ["Bold", 700, false], ["Heavy", 800, false]]),
  family("Georgia", REGULAR_BOLD, { file: "/System/Library/Fonts/Supplemental/Georgia.ttf" }),
  family("Helvetica Neue", [["Light", 300, false], ...REGULAR_BOLD]),
  family("Inter", [["Regular", 400, false], ["Italic", 400, true]], { variable: true, user: true, file: "/Users/preview/Library/Fonts/InterVariable.ttf" }),
  family("Menlo", REGULAR_BOLD, { mono: true }),
  family("New York", [["Regular", 400, false]], { variable: true, file: "/System/Library/Fonts/NewYork.ttf" }),
  family("SF Mono", [["Regular", 400, false], ["Bold", 700, false]], { mono: true }),
  family("SF Pro", [["Regular", 400, false]], { variable: true, file: "/System/Library/Fonts/SFNS.ttf" }),
  family("Times New Roman", REGULAR_BOLD, { file: "/System/Library/Fonts/Supplemental/Times New Roman.ttf" }),
  family("Courier New", REGULAR_BOLD, { mono: true, file: "/System/Library/Fonts/Supplemental/Courier New.ttf" }),
  family("Arial", REGULAR_BOLD, { file: "/System/Library/Fonts/Supplemental/Arial.ttf" }),
];

const cancelled = new Set<string>();
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function label(weight: number, italic: boolean) {
  const names: Record<number, string> = { 100: "Thin", 200: "ExtraLight", 300: "Light", 400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold", 900: "Black" };
  const base = names[weight] ?? "Regular";
  if (!italic) return base;
  return base === "Regular" ? "Italic" : `${base}Italic`;
}

async function install(args: Args) {
  const jobId = String(args.jobId);
  const d = details(String(args.id));
  if (d.license_class !== "open") throw `${d.family} uses the license "${d.license}", which is not on the open list (OFL, Apache, MIT). Read its terms on the author's site before using it.`;
  const subset = String(args.subset ?? "latin");
  const outDir = (args.outDir as string | null) ?? null;
  const dir = outDir ?? "/Users/preview/Library/Fonts";
  const stem = d.family.replace(/[^A-Za-z0-9]/g, "");
  const full = d.source === "google";
  const files = full ? d.files.filter((f) => f.subset === d.default_subset) : d.files.filter((f) => f.subset === subset);
  const outcome = { dir, installed: !outDir, saved: [] as string[], skipped: [] as string[], failed: [] as { file_name: string; error: string }[], cancelled: false, cache_refreshed: false, registered: 0, needs_restart: false, coverage: full ? "full" : "subset" };
  for (let index = 0; index < files.length; index++) {
    if (cancelled.has(jobId)) {
      outcome.cancelled = true;
      break;
    }
    const f = files[index];
    const suffix = full || subset === "latin" ? "" : `-${subset.replace(/(^|-)([a-z])/g, (_m, _d, c: string) => c.toUpperCase())}`;
    const name = `${stem}${suffix}-${label(f.weight, f.style === "italic")}.ttf`;
    if (d.id === "inter" && f.weight === 400 && !outDir) {
      outcome.skipped.push(name);
      await emit("fonts-install-progress", { job_id: jobId, index, total: files.length, file_name: name, state: "skipped" });
      continue;
    }
    await emit("fonts-install-progress", { job_id: jobId, index, total: files.length, file_name: name, state: "downloading" });
    await sleep(140);
    if (d.id === "space-grotesk" && f.weight === 700) {
      const error = "The font CDN answered 503 Service Unavailable for this file.";
      outcome.failed.push({ file_name: name, error });
      await emit("fonts-install-progress", { job_id: jobId, index, total: files.length, file_name: name, state: "failed", error });
      continue;
    }
    outcome.saved.push(`${dir}/${name}`);
    await emit("fonts-install-progress", { job_id: jobId, index, total: files.length, file_name: name, state: "saved" });
  }
  cancelled.delete(jobId);
  return outcome;
}

export const fontsFixtures: Record<string, (args: Args) => unknown> = {
  fonts_installed: async () => {
    await sleep(500);
    return INSTALLED;
  },
  fonts_catalog: async () => {
    await sleep(350);
    return CATALOG.map(catalogFont).sort((a, b) => a.family.localeCompare(b.family));
  },
  fonts_details: async ({ id }) => {
    await sleep(120);
    return details(String(id));
  },
  fonts_user_dir: () => "/Users/preview/Library/Fonts",
  fonts_install: install,
  fonts_cancel: ({ jobId }) => {
    cancelled.add(String(jobId));
    return null;
  },
};

export function fontsDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/fonts")) return undefined;
  if (options?.directory) return "/Users/preview/Downloads/Fonts";
  return null;
}
