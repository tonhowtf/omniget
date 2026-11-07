type Args = Record<string, unknown>;

type PreviewSet = { prefix: string; name: string; total: number; author: string; spdx: string; license: string; category: string; stroke: boolean };

const SETS: PreviewSet[] = [
  { prefix: "lucide", name: "Lucide", total: 2020, author: "Lucide Contributors", spdx: "ISC", license: "ISC", category: "UI 24px", stroke: true },
  { prefix: "ph", name: "Phosphor", total: 9072, author: "Phosphor Icons", spdx: "MIT", license: "MIT", category: "UI Other / Mixed Grid", stroke: false },
  { prefix: "tabler", name: "Tabler Icons", total: 6257, author: "Paweł Kuna", spdx: "MIT", license: "MIT", category: "UI 24px", stroke: true },
  { prefix: "hugeicons", name: "Huge Icons", total: 6065, author: "Hugeicons", spdx: "MIT", license: "MIT", category: "UI 24px", stroke: true },
  { prefix: "heroicons", name: "HeroIcons", total: 1288, author: "Refactoring UI Inc", spdx: "MIT", license: "MIT", category: "UI Other / Mixed Grid", stroke: false },
  { prefix: "material-symbols", name: "Material Symbols", total: 15761, author: "Google", spdx: "Apache-2.0", license: "Apache 2.0", category: "Material", stroke: false },
  { prefix: "simple-icons", name: "Simple Icons", total: 3467, author: "Simple Icons Collaborators", spdx: "CC0-1.0", license: "CC0 1.0", category: "Logos", stroke: false },
  { prefix: "iconoir", name: "Iconoir", total: 1671, author: "Luca Burgio", spdx: "MIT", license: "MIT", category: "UI 24px", stroke: true },
  { prefix: "ri", name: "Remix Icon", total: 3188, author: "Remix Design", spdx: "Apache-2.0", license: "Apache 2.0", category: "UI 24px", stroke: false },
  { prefix: "mdi", name: "Material Design Icons", total: 7447, author: "Pictogrammers", spdx: "Apache-2.0", license: "Apache 2.0", category: "Material", stroke: false },
  { prefix: "game-icons", name: "Game Icons", total: 4123, author: "GameIcons", spdx: "CC-BY-3.0", license: "CC BY 3.0", category: "Thematic", stroke: false },
  { prefix: "fluent-emoji", name: "Fluent Emoji", total: 3126, author: "Microsoft Corporation", spdx: "MIT", license: "MIT", category: "Emoji", stroke: false },
  { prefix: "flag", name: "Flag Icons", total: 542, author: "Panayiotis Lipiridis", spdx: "MIT", license: "MIT", category: "Flags", stroke: false },
  { prefix: "devicon", name: "Devicon", total: 900, author: "konpa", spdx: "MIT", license: "MIT", category: "Programming", stroke: false },
  { prefix: "noto-v1", name: "Noto Emoji v1", total: 2162, author: "Google Inc", spdx: "Apache-2.0", license: "Apache 2.0", category: "Archive / Unmaintained", stroke: false },
  { prefix: "pepicons", name: "Pepicons", total: 428, author: "CyCraft", spdx: "CC-BY-4.0", license: "CC BY 4.0", category: "UI Other / Mixed Grid", stroke: false },
  { prefix: "academicons", name: "Academicons", total: 158, author: "James Walsh", spdx: "OFL-1.1", license: "Open Font License", category: "Thematic", stroke: false },
  { prefix: "uil", name: "Unicons", total: 1215, author: "Iconscout", spdx: "Apache-2.0", license: "Apache 2.0", category: "UI 24px", stroke: false },
  { prefix: "nonfree", name: "Sample NC Set", total: 120, author: "Example", spdx: "CC-BY-NC-4.0", license: "CC BY-NC 4.0", category: "High Detail", stroke: false },
];

const WORDS = [
  "house", "home", "arrow-left", "arrow-right", "arrow-up", "arrow-down", "arrow-up-right", "arrow-circle-left", "arrows-clockwise", "star", "star-half",
  "heart", "user", "users", "user-plus", "settings", "gear", "search", "bell", "bell-off", "download", "upload", "cloud", "cloud-download", "camera",
  "image", "file", "file-text", "folder", "folder-open", "trash", "pencil", "copy", "clipboard", "check", "check-circle", "x", "x-circle", "plus",
  "minus", "lock", "unlock", "key", "mail", "send", "message", "phone", "calendar", "clock", "map-pin", "globe", "link", "share", "bookmark",
  "play", "pause", "stop", "music", "video", "mic", "headphones", "sun", "moon", "zap", "flame", "leaf", "shopping-cart", "credit-card", "gift",
  "chart-bar", "chart-line", "database", "server", "terminal", "code", "git-branch", "bug", "shield", "eye", "eye-off", "filter", "sliders", "menu",
  "grid", "list", "layout", "home-2", "home-heart", "home-search",
];

const BRANDS = ["github", "gitlab", "figma", "svelte", "react", "rust", "tauri", "discord", "youtube", "spotify", "apple", "android"];

function hash(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return h >>> 0;
}

const SHAPES = [
  "M4 12h16M12 4v16",
  "M5 19 19 5M9 5h10v10",
  "M3 10.5 12 3l9 7.5V21H3z",
  "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8v4l3 2",
  "M4 6h16M4 12h16M4 18h10",
  "M6 4h9l3 3v13H6zM9 12h6M9 16h6",
  "M12 3l2.6 5.6 6.1.7-4.5 4.2 1.2 6L12 16.8 6.6 19.5l1.2-6L3.3 9.3l6.1-.7z",
  "M3 7h18v12H3zM3 7l9 6 9-6",
  "M12 21s-7-4.5-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 11c0 5.5-7 10-7 10z",
  "M5 12l4 4 10-10",
];

function body(prefix: string, name: string, stroke: boolean): string {
  if (prefix === "lucide" && name === "house") {
    return '<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></g>';
  }
  const h = hash(`${prefix}:${name}`);
  const a = SHAPES[h % SHAPES.length];
  const b = SHAPES[(h >> 5) % SHAPES.length];
  if (stroke) {
    return `<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="${a}"/>${h % 3 === 0 ? `<path d="${b}" opacity=".5"/>` : ""}</g>`;
  }
  return `<path fill="currentColor" fill-rule="evenodd" d="${a}" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/>${h % 2 ? `<circle cx="18" cy="6" r="3" fill="currentColor"/>` : ""}`;
}

function namesOf(set: PreviewSet): string[] {
  if (set.prefix === "simple-icons" || set.prefix === "devicon") return BRANDS;
  if (set.prefix === "ph") return WORDS.flatMap((w) => [w, `${w}-bold`, `${w}-fill`, `${w}-duotone`]);
  if (set.prefix === "heroicons") return WORDS.flatMap((w) => [w, `${w}-solid`, `${w}-20-solid`]);
  return WORDS;
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function collections() {
  return {
    sets: SETS.map((s) => ({
      prefix: s.prefix,
      name: s.name,
      total: s.total,
      author: s.author,
      author_url: null,
      license: s.license,
      license_spdx: s.spdx,
      license_url: null,
      category: s.category,
      palette: s.category === "Emoji" || s.category === "Flags",
      height: 24,
      samples: namesOf(s).slice(0, 3),
      tags: s.stroke ? ["Uses Stroke"] : [],
    })),
    offline: false,
    fetched_at: Math.floor(Date.now() / 1000),
  };
}

async function search(args: Args) {
  const query = String(args.query ?? "").trim().toLowerCase();
  const prefixes = (args.prefixes as string[] | undefined) ?? [];
  const start = Number(args.start ?? 0);
  await sleep(query.length > 0 ? 380 : 0);
  if (query === "offline") throw { kind: "offline", code: "unreachable", params: {}, message: "Could not reach the icon server: connection refused." };
  if (!query) throw { kind: "invalid", code: "query_empty", params: {}, message: "Type something to search." };
  const scope = prefixes.length ? SETS.filter((s) => prefixes.includes(s.prefix)) : SETS;
  const terms = query.split(/\s+/);
  const icons: string[] = [];
  const more: string[] = [];
  for (const set of scope.slice(0, 12)) {
    const hits = namesOf(set).filter((n) => terms.every((t) => n.includes(t)));
    const page = hits.slice(start, start + 128);
    if (page.length === 128 || (query === "a" && start === 0)) more.push(set.prefix);
    icons.push(...page.map((n) => `${set.prefix}:${n}`));
  }
  return { icons, capped: more.length > 0, more, next_start: start + 128, trimmed: Math.max(0, prefixes.length - 12), offline: false };
}

async function getIcons(args: Args) {
  const prefix = String(args.prefix);
  const names = (args.names as string[]) ?? [];
  const set = SETS.find((s) => s.prefix === prefix);
  await sleep(120 + Math.random() * 160);
  if (!set) return { prefix, icons: [], missing: names, offline: false };
  return {
    prefix,
    icons: names.map((name) => ({ name, body: body(prefix, name, set.stroke), width: 24, height: 24, left: 0, top: 0, rotate: 0, h_flip: false, v_flip: false })),
    missing: [],
    offline: false,
  };
}

function saved(prefix: string, name: string) {
  const set = SETS.find((s) => s.prefix === prefix)!;
  return {
    prefix,
    icon: { name, body: body(prefix, name, set.stroke), width: 24, height: 24, left: 0, top: 0, rotate: 0, h_flip: false, v_flip: false },
    set_name: set.name,
    license: set.license,
    license_spdx: set.spdx,
    added_at: Math.floor(Date.now() / 1000),
  };
}

let library = {
  version: 1,
  favorites: [saved("lucide", "house"), saved("ph", "heart-fill"), saved("tabler", "download")],
  collections: [
    { id: "preview-marketing", name: "Marketing site", created_at: 1, icons: [saved("lucide", "arrow-right"), saved("lucide", "check"), saved("simple-icons", "github"), saved("game-icons", "shield")] },
  ],
};

export const iconsFixtures: Record<string, (args: Args) => unknown> = {
  icons_collections: () => collections(),
  icons_search: search,
  icons_get: getIcons,
  icons_library: () => structuredClone(library),
  icons_library_save: ({ library: next }) => {
    library = structuredClone(next as typeof library);
    return structuredClone(library);
  },
  icons_cache_info: () => ({ bytes: 1_840_000 }),
  icons_cache_clear: () => null,
  icons_save_file: async ({ kind, fileName }) => {
    await sleep(300);
    return { path: `/preview/Downloads/${String(fileName)}.${String(kind)}`, count: 1 };
  },
  icons_export: async ({ mode, name, files }) => {
    await sleep(400);
    const count = (files as unknown[]).length;
    return mode === "sprite" ? { path: `/preview/Downloads/${String(name)}-sprite.svg`, count } : { path: `/preview/Downloads/${String(name)}`, count };
  },
};

export function iconsDialog(_options: Record<string, unknown> | undefined): unknown {
  return undefined;
}
