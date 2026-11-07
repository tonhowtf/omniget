import type { ItemDetail, ItemFile, ItemSummary, ParsedRegistry, RegistrySource } from "./types";

export const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
export const HIDDEN_TYPES = new Set(["example", "internal", "style", "font"]);
const MAX_TEXT = 2_000;
const MAX_LIST = 200;
const MAX_FILE = 1_000_000;

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown, max = MAX_TEXT): string {
  return typeof v === "string" ? v.slice(0, max).trim() : "";
}

function strList(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  const out = new Set<string>();
  for (const x of v) {
    if (typeof x === "string" && x.trim() && out.size < MAX_LIST) out.add(x.trim().slice(0, 300));
  }
  return [...out];
}

export function shortType(raw: unknown): string {
  const s = str(raw, 80).toLowerCase();
  if (!s) return "ui";
  const cut = s.startsWith("registry:") ? s.slice(9) : s;
  return /^[a-z][a-z0-9-]*$/.test(cut) ? cut : "other";
}

function httpsUrl(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  try {
    const u = new URL(raw.trim());
    return u.protocol === "https:" ? u.toString() : null;
  } catch {
    return null;
  }
}

export function parseRegistry(body: unknown, registryId: string): ParsedRegistry {
  let rawItems: unknown[] = [];
  let name: string | null = null;
  let homepage: string | null = null;
  if (Array.isArray(body)) rawItems = body;
  else if (isObject(body)) {
    if (!Array.isArray(body.items)) throw new Error("shape");
    rawItems = body.items;
    name = str(body.name, 120) || null;
    homepage = httpsUrl(body.homepage);
  } else {
    throw new Error("shape");
  }
  const items: ItemSummary[] = [];
  const seen = new Set<string>();
  let skipped = 0;
  for (const raw of rawItems) {
    if (!isObject(raw)) {
      skipped++;
      continue;
    }
    const itemName = str(raw.name, 128);
    if (!NAME_RE.test(itemName) || seen.has(itemName)) {
      skipped++;
      continue;
    }
    seen.add(itemName);
    items.push({
      key: `${registryId}::${itemName}`,
      registryId,
      name: itemName,
      title: str(raw.title, 200) || itemName,
      description: str(raw.description),
      type: shortType(raw.type),
      categories: strList(raw.categories),
      dependencies: strList(raw.dependencies),
      registryDependencies: strList(raw.registryDependencies),
      fileCount: Array.isArray(raw.files) ? raw.files.length : 0,
    });
  }
  return { name, homepage, items, skipped };
}

export function parseItem(body: unknown): ItemDetail {
  if (!isObject(body)) throw new Error("shape");
  const name = str(body.name, 128);
  if (!NAME_RE.test(name)) throw new Error("shape");
  const files: ItemFile[] = [];
  if (Array.isArray(body.files)) {
    for (const f of body.files) {
      if (!isObject(f)) continue;
      const path = str(f.path, 400);
      if (!path || files.some((x) => x.path === path)) continue;
      files.push({
        path,
        type: shortType(f.type),
        target: str(f.target, 400) || null,
        content: typeof f.content === "string" ? f.content.slice(0, MAX_FILE) : "",
      });
    }
  }
  let cssVarCount = 0;
  if (isObject(body.cssVars)) {
    for (const group of Object.values(body.cssVars)) if (isObject(group)) cssVarCount += Object.keys(group).length;
  }
  return {
    name,
    title: str(body.title, 200) || name,
    description: str(body.description),
    type: shortType(body.type),
    dependencies: strList(body.dependencies),
    devDependencies: strList(body.devDependencies),
    registryDependencies: strList(body.registryDependencies),
    files,
    cssVarCount,
  };
}

export function itemUrl(source: RegistrySource, name: string): string | null {
  if (!NAME_RE.test(name) || !source.item_url.includes("{name}")) return null;
  return httpsUrl(source.item_url.replace("{name}", encodeURIComponent(name)));
}

export function docsUrl(source: RegistrySource, item: { name: string; type: string }): string | null {
  if (!source.docs_url || !NAME_RE.test(item.name)) return null;
  if (!["ui", "component", "block"].includes(item.type)) return null;
  if (item.type === "block" && source.id === "shadcn") return httpsUrl(`${source.homepage.replace(/\/$/, "")}/blocks`);
  return httpsUrl(source.docs_url.replace("{name}", encodeURIComponent(item.name)));
}

export function customSource(raw: string): RegistrySource | null {
  const url = httpsUrl(raw);
  if (!url) return null;
  const u = new URL(url);
  if (!u.pathname.endsWith(".json")) return null;
  const host = u.hostname;
  if (host === "localhost" || host.endsWith(".local") || /^[\d.]+$/.test(host) || host.startsWith("[")) return null;
  const dir = url.slice(0, url.lastIndexOf("/") + 1);
  return {
    id: `custom:${url}`,
    name: host.replace(/^www\./, ""),
    homepage: `${u.protocol}//${u.host}`,
    index_url: url,
    item_url: `${dir}{name}.json`,
    docs_url: null,
    custom: true,
  };
}
