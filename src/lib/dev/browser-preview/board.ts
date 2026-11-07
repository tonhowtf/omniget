type Args = Record<string, unknown>;

type Stored = { name: string; contents: string; modified: number; thumb: string | null };

const DEFAULT_FOLDER = "/preview/Library/Application Support/omniget/board";
const OTHER_FOLDER = "/preview/Documents/Whiteboards";
const DAY = 86_400_000;

let folder = DEFAULT_FOLDER;
const folders = new Map<string, Map<string, Stored>>();
const external = new Map<string, Stored>();
const parked = new Map<string, Stored>();

function scene(elements: Record<string, unknown>[]): string {
  return JSON.stringify({ type: "excalidraw", version: 2, source: "omniget", elements, appState: { viewBackgroundColor: "#ffffff", gridSize: 20 }, files: {} });
}

function rect(id: string, x: number, y: number, w: number, h: number, bg: string): Record<string, unknown> {
  return { id, type: "rectangle", x, y, width: w, height: h, strokeColor: "#1e1e1e", backgroundColor: bg, fillStyle: "solid", roughness: 1, strokeWidth: 2, roundness: { type: 3 }, version: 1, seed: id.length * 7 };
}

function text(id: string, x: number, y: number, value: string): Record<string, unknown> {
  return { id, type: "text", x, y, width: value.length * 11, height: 25, text: value, originalText: value, fontSize: 20, fontFamily: 5, strokeColor: "#1e1e1e", version: 1, seed: id.length * 11 };
}

function arrow(id: string, x: number, y: number, dx: number): Record<string, unknown> {
  return { id, type: "arrow", x, y, width: dx, height: 0, points: [[0, 0], [dx, 0]], strokeColor: "#1e1e1e", strokeWidth: 2, endArrowhead: "arrow", version: 1, seed: id.length * 13 };
}

function seed(): Map<string, Stored> {
  const now = Date.now();
  return new Map<string, Stored>([
    [`${DEFAULT_FOLDER}/Release plan.excalidraw`, { name: "Release plan", modified: now - 4 * 60_000, thumb: null, contents: scene([rect("a", 0, 0, 180, 90, "#ffec99"), text("at", 30, 30, "Bump version"), arrow("ar", 190, 45, 120), rect("b", 320, 0, 180, 90, "#b2f2bb"), text("bt", 360, 30, "Tag + build")]) }],
    [`${DEFAULT_FOLDER}/Fluxo de onboarding.excalidraw`, { name: "Fluxo de onboarding", modified: now - 3 * 3_600_000, thumb: null, contents: scene([rect("c", 0, 0, 220, 120, "#a5d8ff"), text("ct", 20, 45, "Tela de boas-vindas")]) }],
    [`${DEFAULT_FOLDER}/Arquitetura do núcleo – rascunho com nome bem comprido para testar quebra.excalidraw`, { name: "Arquitetura do núcleo – rascunho com nome bem comprido para testar quebra", modified: now - 9 * DAY, thumb: null, contents: scene([]) }],
  ]);
}

function store(): Map<string, Stored> {
  let map = folders.get(folder);
  if (!map) {
    map = folder === DEFAULT_FOLDER ? seed() : new Map();
    folders.set(folder, map);
  }
  return map;
}

function entry(path: string, s: Stored) {
  return { name: s.name, path, modified_ms: s.modified, size: s.contents.length, has_thumb: s.thumb !== null };
}

function lib() {
  const boards = [...store().entries()].map(([p, s]) => entry(p, s)).sort((a, b) => b.modified_ms - a.modified_ms);
  return { folder, is_default: folder === DEFAULT_FOLDER, boards };
}

function find(path: string): { map: Map<string, Stored>; s: Stored } {
  const map = store();
  const s = map.get(path);
  if (s) return { map, s };
  const e = external.get(path);
  if (e) return { map: external, s: e };
  throw "BOARD_OUTSIDE";
}

function checkName(raw: string): string {
  const name = raw.trim().replace(/\.excalidraw$/i, "").trim();
  if (!name) throw "BOARD_NAME_EMPTY";
  if (/[/\\:*?"<>|]/.test(name) || name.startsWith(".") || name.endsWith(".")) throw "BOARD_NAME_INVALID";
  if ([...name].length > 120) throw "BOARD_NAME_TOO_LONG";
  return name;
}

function taken(map: Map<string, Stored>, name: string): boolean {
  return [...map.values()].some((s) => s.name.toLowerCase() === name.toLowerCase());
}

function unique(map: Map<string, Stored>, base: string): string {
  if (!taken(map, base)) return base;
  for (let n = 2; ; n++) if (!taken(map, `${base} ${n}`)) return `${base} ${n}`;
}

function add(name: string, contents: string) {
  const map = store();
  const final = unique(map, name);
  const path = `${folder}/${final}.excalidraw`;
  const s = { name: final, contents, modified: Date.now(), thumb: null };
  map.set(path, s);
  return entry(path, s);
}

function dirOf(path: string): string {
  return path.slice(0, path.lastIndexOf("/"));
}

export const boardFixtures: Record<string, (args: Args) => unknown> = {
  board_library: () => lib(),
  board_choose_folder: () => {
    folder = folder === DEFAULT_FOLDER ? OTHER_FOLDER : DEFAULT_FOLDER;
    return lib();
  },
  board_reset_folder: () => {
    folder = DEFAULT_FOLDER;
    return lib();
  },
  board_create: ({ name, fallback }) => add(checkName(String(name ?? fallback ?? "Untitled")), scene([])),
  board_read: ({ path }) => {
    const { map, s } = find(String(path));
    return { path, name: s.name, contents: s.contents, external: map === external, modified_ms: s.modified };
  },
  board_open_file: () => {
    const path = "/preview/Downloads/Sprint retro.excalidraw";
    if (!external.has(path)) external.set(path, { name: "Sprint retro", modified: Date.now() - 2 * DAY, thumb: null, contents: scene([rect("r", 0, 0, 160, 160, "#ffc9c9"), text("rt", 20, 65, "What hurt?")]) });
    const s = external.get(path)!;
    return { path, name: s.name, contents: s.contents, external: true, modified_ms: s.modified };
  },
  board_save: ({ path, contents, thumbnail }) => {
    const p = String(path);
    if (p.includes("fail")) throw "BOARD_IO: Permission denied (os error 13)";
    const { s } = find(p);
    const text = String(contents ?? "");
    try {
      const parsed = JSON.parse(text);
      if (!Array.isArray(parsed.elements)) throw new Error();
    } catch {
      throw "BOARD_NOT_SCENE";
    }
    s.contents = text;
    s.modified = Date.now();
    if (typeof thumbnail === "string" && thumbnail) s.thumb = thumbnail;
    return { modified_ms: s.modified, size: text.length };
  },
  board_rename: ({ path, newName }) => {
    const p = String(path);
    const { map, s } = find(p);
    const name = checkName(String(newName ?? ""));
    if (name.toLowerCase() !== s.name.toLowerCase() && taken(map, name)) throw "BOARD_EXISTS";
    map.delete(p);
    s.name = name;
    const next = `${dirOf(p)}/${name}.excalidraw`;
    map.set(next, s);
    return entry(next, s);
  },
  board_duplicate: ({ path, suffix }) => {
    const { s } = find(String(path));
    const created = add(`${s.name} ${String(suffix ?? "copy")}`, s.contents);
    const copy = store().get(created.path);
    if (copy) copy.thumb = s.thumb;
    return { ...created, has_thumb: s.thumb !== null };
  },
  board_delete: ({ path }) => {
    const p = String(path);
    const map = store();
    const s = map.get(p);
    if (!s) throw "BOARD_NOT_FOUND";
    map.delete(p);
    const park = `${folder}/.trash/${Date.now()}-${s.name}.excalidraw`;
    parked.set(park, s);
    return { method: "folder", restore_path: park };
  },
  board_restore: ({ restorePath }) => {
    const s = parked.get(String(restorePath));
    if (!s) throw "BOARD_NOT_FOUND";
    parked.delete(String(restorePath));
    const created = add(s.name, s.contents);
    return created;
  },
  board_import: () => [add("Imported mind map", scene([rect("m", 0, 0, 140, 70, "#d0bfff"), text("mt", 20, 22, "Central idea")]))],
  board_thumbnail: ({ path }) => {
    const { s } = find(String(path));
    return s.thumb ? `data:image/png;base64,${s.thumb}` : null;
  },
  board_export: ({ fileName, kind }) => `/preview/Exports/${String(fileName)}.${kind === "excalidraw" ? "excalidraw" : String(kind)}`,
  board_copy_png: () => {
    throw "BOARD_CLIPBOARD: preview has no native clipboard";
  },
};

export function boardDialog(_options: Record<string, unknown> | undefined): unknown {
  return undefined;
}
