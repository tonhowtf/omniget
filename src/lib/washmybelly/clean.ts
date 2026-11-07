import type { Risk } from "./api";
import type { SelectItem } from "./ui";

export type JunkItem = {
  id: string;
  rule_id: string;
  label: string;
  category: string;
  path: string;
  bytes: number;
  files: number;
  risk: Risk;
  admin: boolean;
  destination: "unlink" | "quarantine" | "trash" | string;
  running: string | null;
  selected_by_default: boolean;
};

export type JunkScan = {
  items: JunkItem[];
  total_bytes: number;
  selected_bytes: number;
  plan_id: string | null;
  categories: { id: string; bytes: number; count: number }[];
  skipped_rules: number;
  protected_paths: number;
  needs_full_disk_access: boolean;
};

const CATEGORY_ORDER = [
  "system",
  "apps",
  "browser",
  "dev",
  "package_cache",
  "shader_cache",
  "games",
  "logs",
  "database",
  "trash",
  "user_files",
  "privacy",
] as const;

const CATEGORY_COLOR: Record<string, string> = {
  system: "var(--accent, #0a84ff)",
  apps: "var(--info, #64d2ff)",
  browser: "#5e5ce6",
  dev: "#30b0c7",
  package_cache: "#ac8e68",
  shader_cache: "#bf5af2",
  games: "#ff375f",
  logs: "var(--warning, #ff9f0a)",
  database: "#32d74b",
  trash: "#8e8e93",
  user_files: "#ffd60a",
  privacy: "var(--danger, #ff453a)",
};

function categoryColor(id: string): string {
  return CATEGORY_COLOR[id] ?? "#98989d";
}

export function categoryKey(id: string): string {
  return `washmybelly.clean.category.${id}`;
}

export function ruleLabelKey(ruleId: string): string {
  return `washmybelly.clean.rule.${ruleId.replace(/\./g, "_")}`;
}

function orderOf(id: string): number {
  const i = (CATEGORY_ORDER as readonly string[]).indexOf(id);
  return i < 0 ? CATEGORY_ORDER.length : i;
}

type RunningReason =
  | { kind: "in_use"; who: string }
  | { kind: "close"; app: string }
  | { kind: "lock" };

export function runningReason(running: string | null | undefined): RunningReason | null {
  if (!running) return null;
  const inUse = /^in use by (.+)$/i.exec(running.trim());
  if (inUse) return { kind: "in_use", who: inUse[1] };
  if (/[\\/]/.test(running) || /^[~$%]/.test(running)) return { kind: "lock" };
  const base = running.replace(/\.exe$/i, "");
  return { kind: "close", app: prettyApp(base) };
}

const APP_NAMES: Record<string, string> = {
  firefox: "Firefox",
  "google chrome": "Google Chrome",
  chrome: "Google Chrome",
  msedge: "Microsoft Edge",
  brave: "Brave",
  "brave browser": "Brave",
  safari: "Safari",
  opera: "Opera",
  vivaldi: "Vivaldi",
  thunderbird: "Thunderbird",
  steam: "Steam",
  code: "VS Code",
  explorer: "Explorer",
};

function prettyApp(name: string): string {
  const known = APP_NAMES[name.toLowerCase()];
  if (known) return known;
  return name.length ? name[0].toUpperCase() + name.slice(1) : name;
}

export function defaultJunkSelection(items: readonly JunkItem[]): Set<string> {
  return new Set(items.filter((i) => i.selected_by_default && !i.running).map((i) => i.id));
}

function isBlocked(item: JunkItem): boolean {
  return !!item.running;
}

export type CategorySlice = { id: string; bytes: number; count: number; fraction: number; color: string };

export function categorySlices(scan: Pick<JunkScan, "categories" | "items">): CategorySlice[] {
  let cats = scan.categories?.length
    ? scan.categories.map((c) => ({ ...c }))
    : aggregate(scan.items ?? []);
  cats = cats.filter((c) => c.count > 0);
  const total = cats.reduce((s, c) => s + c.bytes, 0);
  return cats
    .sort((a, b) => b.bytes - a.bytes || orderOf(a.id) - orderOf(b.id))
    .map((c) => ({ ...c, fraction: total > 0 ? c.bytes / total : 0, color: categoryColor(c.id) }));
}

function aggregate(items: readonly JunkItem[]): { id: string; bytes: number; count: number }[] {
  const m = new Map<string, { id: string; bytes: number; count: number }>();
  for (const it of items) {
    const c = m.get(it.category) ?? { id: it.category, bytes: 0, count: 0 };
    c.bytes += it.bytes;
    c.count++;
    m.set(it.category, c);
  }
  return [...m.values()];
}

export function selectedBytes(items: readonly { id: string; bytes?: number }[], selected: ReadonlySet<string>): number {
  let n = 0;
  for (const it of items) if (selected.has(it.id)) n += it.bytes ?? 0;
  return n;
}

export function selectedByCategory(items: readonly JunkItem[], selected: ReadonlySet<string>): Map<string, number> {
  const m = new Map<string, number>();
  for (const it of items) if (selected.has(it.id)) m.set(it.category, (m.get(it.category) ?? 0) + it.bytes);
  return m;
}

export function junkRows(
  items: readonly JunkItem[],
  label: (it: JunkItem) => string,
  hint: (it: JunkItem) => string | null,
): (SelectItem & { junk: JunkItem })[] {
  const weight = new Map<string, number>();
  for (const it of items) weight.set(it.category, (weight.get(it.category) ?? 0) + it.bytes);
  return [...items]
    .sort(
      (a, b) =>
        (weight.get(b.category) ?? 0) - (weight.get(a.category) ?? 0) ||
        orderOf(a.category) - orderOf(b.category) ||
        a.category.localeCompare(b.category) ||
        b.bytes - a.bytes,
    )
    .map((it) => ({
      id: it.id,
      label: label(it),
      detail: it.path.startsWith("/") || /^[A-Za-z]:\\/.test(it.path) || it.path.startsWith("~") ? it.path : null,
      detailIsPath: true,
      hint: hint(it),
      bytes: it.bytes,
      risk: it.risk,
      group: it.category,
      disabled: isBlocked(it),
      junk: it,
    }));
}

export function pruneSelection(selected: ReadonlySet<string>, items: readonly { id: string; disabled?: boolean }[]): Set<string> {
  const ok = new Set(items.filter((i) => !i.disabled).map((i) => i.id));
  return new Set([...selected].filter((id) => ok.has(id)));
}

type ArtifactItem = {
  id: string;
  label: string;
  detail?: string | null;
  group: string;
  bytes: number;
  risk: Risk;
};

type ArtifactProject = { path: string; kind: string; kinds: string[]; bytes: number; age_days: number };

export type ArtifactScan = {
  items: ArtifactItem[];
  total_bytes: number;
  plan_id: string | null;
  projects: ArtifactProject[];
  roots: string[];
  skipped_roots: string[];
};

function artifactPath(id: string): string {
  const m = /^dev_artifacts:[^:]+:(.+)$/.exec(id);
  return m ? m[1] : id;
}

function isInside(child: string, parent: string): boolean {
  if (child === parent) return true;
  const p = parent.endsWith("/") || parent.endsWith("\\") ? parent : parent + (parent.includes("\\") && !parent.includes("/") ? "\\" : "/");
  return child.startsWith(p);
}

function projectOf(id: string, projects: readonly ArtifactProject[]): ArtifactProject | null {
  const path = artifactPath(id);
  let best: ArtifactProject | null = null;
  for (const p of projects) {
    if (isInside(path, p.path) && (!best || p.path.length > best.path.length)) best = p;
  }
  return best;
}

export function baseName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function artifactRows(scan: Pick<ArtifactScan, "items" | "projects">): SelectItem[] {
  const rows = scan.items.map((it) => {
    const proj = projectOf(it.id, scan.projects);
    const path = artifactPath(it.id);
    const rel = proj && path !== proj.path ? path.slice(proj.path.length).replace(/^[\\/]/, "") : baseName(path);
    return {
      id: it.id,
      label: rel,
      hint: it.group,
      bytes: it.bytes,
      risk: it.risk,
      group: proj?.path ?? "",
      projectBytes: proj?.bytes ?? it.bytes,
    };
  });
  rows.sort((a, b) => b.projectBytes - a.projectBytes || a.group.localeCompare(b.group) || b.bytes - a.bytes);
  return rows.map(({ projectBytes: _p, ...r }) => r);
}

type DupFile = { path: string; mtime: number; keep: boolean };
export type DupGroup = { hash: string; size: number; files: DupFile[]; wasted: number };

export type DupScan = {
  groups: DupGroup[];
  groups_total: number;
  total_bytes: number;
  plan_id: string | null;
  clone_plan_id: string | null;
  scanned_files: number;
  cache_hits: number;
  roots: string[];
  skipped_roots: string[];
};

export const trashId = (path: string) => `duplicates:trash:${path}`;
const cloneId = (path: string) => `duplicates:clone:${path}`;

export function initialKeeps(groups: readonly DupGroup[]): Map<string, string> {
  const m = new Map<string, string>();
  for (const g of groups) {
    const k = g.files.find((f) => f.keep) ?? g.files[0];
    if (k) m.set(g.hash, k.path);
  }
  return m;
}

export function defaultDupSelection(groups: readonly DupGroup[], keeps: ReadonlyMap<string, string>): Set<string> {
  const s = new Set<string>();
  for (const g of groups) for (const f of g.files) if (f.path !== keeps.get(g.hash)) s.add(trashId(f.path));
  return s;
}

export function setKeep(
  groups: readonly DupGroup[],
  keeps: ReadonlyMap<string, string>,
  selected: ReadonlySet<string>,
  hash: string,
  path: string,
): { keeps: Map<string, string>; selected: Set<string> } {
  const g = groups.find((x) => x.hash === hash);
  const nextKeeps = new Map(keeps);
  const nextSel = new Set(selected);
  if (!g || !g.files.some((f) => f.path === path)) return { keeps: nextKeeps, selected: nextSel };
  nextKeeps.set(hash, path);
  for (const f of g.files) {
    if (f.path === path) nextSel.delete(trashId(f.path));
    else nextSel.add(trashId(f.path));
  }
  return { keeps: nextKeeps, selected: nextSel };
}

export function toggleCopy(selected: ReadonlySet<string>, path: string): Set<string> {
  const s = new Set(selected);
  const id = trashId(path);
  if (s.has(id)) s.delete(id);
  else s.add(id);
  return s;
}

export function dupSelectedBytes(groups: readonly DupGroup[], selected: ReadonlySet<string>): number {
  let n = 0;
  for (const g of groups) for (const f of g.files) if (selected.has(trashId(f.path))) n += g.size;
  return n;
}

export function dupSelectedCount(groups: readonly DupGroup[], selected: ReadonlySet<string>): number {
  let n = 0;
  for (const g of groups) for (const f of g.files) if (selected.has(trashId(f.path))) n++;
  return n;
}

export function dupPlanIds(groups: readonly DupGroup[], selected: ReadonlySet<string>): { ids: string[]; heldBack: number } {
  const ids: string[] = [];
  const heldBack = 0;
  for (const g of groups)
    for (const f of g.files) {
      if (!selected.has(trashId(f.path))) continue;
      ids.push(trashId(f.path));
    }
  return { ids, heldBack };
}

function keepsOriginal(g: DupGroup, keeps: ReadonlyMap<string, string>): boolean {
  const orig = g.files.find((f) => f.keep)?.path;
  return !!orig && keeps.get(g.hash) === orig;
}

export function cloneIdsFor(groups: readonly DupGroup[], keeps: ReadonlyMap<string, string>, selected: ReadonlySet<string>): string[] {
  const out: string[] = [];
  for (const g of groups) {
    if (!keepsOriginal(g, keeps)) continue;
    for (const f of g.files) if (!f.keep && selected.has(trashId(f.path))) out.push(cloneId(f.path));
  }
  return out;
}

export function unclonableCount(groups: readonly DupGroup[], keeps: ReadonlyMap<string, string>, selected: ReadonlySet<string>): number {
  let n = 0;
  for (const g of groups) {
    if (keepsOriginal(g, keeps)) continue;
    for (const f of g.files) if (selected.has(trashId(f.path))) n++;
  }
  return n;
}

export type EmptyScan = {
  items: ArtifactItem[];
  total_bytes: number;
  plan_id: string | null;
  broken_links: { path: string; target: string }[];
};

export function emptyRows(items: readonly ArtifactItem[]): SelectItem[] {
  return items
    .map((it) => {
      const path = it.detail ?? it.id.replace(/^empty_folders:/, "");
      const parent = path.replace(/[\\/][^\\/]*$/, "") || path;
      return { id: it.id, label: it.label || baseName(path), detail: path, detailIsPath: true, bytes: it.bytes, risk: it.risk, group: parent };
    })
    .sort((a, b) => a.group.localeCompare(b.group) || a.label.localeCompare(b.label));
}

export function addRoots(roots: readonly string[], picked: string | string[] | null | undefined): string[] {
  const more = picked == null ? [] : Array.isArray(picked) ? picked : [picked];
  const out = [...roots];
  for (const p of more) if (p && !out.includes(p)) out.push(p);
  return out;
}
