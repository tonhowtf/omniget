/**
 * Catálogo da Central: ponte tipada para os comandos `catalog_*` e
 * `agentkit_*` (Rust: `omniget_core::core::{catalog, agentkit}`) + helpers
 * puros que as telas `/llm/catalog` e `/llm/installed` usam.
 *
 * Nada aqui roda em repouso: cada função só chama o backend quando a tela
 * pede. Os caches são por sessão (memória do módulo).
 */
import { invoke } from "@tauri-apps/api/core";

// ------------------------------------------------------------------ tipos

export type CatalogKind =
  | "agent"
  | "command"
  | "skill"
  | "mcp"
  | "hook"
  | "setting"
  | "statusline"
  | "loop"
  | "workflow"
  | "mod"
  | "plugin"
  | "rule"
  | "template"
  | "sandbox";

export type SortKey = "relevance" | "name" | "stars" | "updated";
export type Scope = "project" | "global" | "local" | "managed";
export type Policy = "rename" | "skip" | "overwrite";

export interface ItemSource {
  id: string;
  repo?: string | null;
  commit?: string | null;
  ref?: string | null;
  path: string;
  dir: string;
  url?: string | null;
  marketplace?: string | null;
  marketplace_name?: string | null;
  upstream?: string | null;
  local?: string | null;
}

export interface CatalogItem {
  id: string;
  kind: CatalogKind | string;
  name: string;
  category: string;
  description: string;
  source: ItemSource;
  license?: string | null;
  author?: string | null;
  tags: string[];
  origin_tool: string;
  files: { path: string; sha256: string; size: number }[];
  entry: string;
  frontmatter: unknown;
  references: string[];
  stars?: number | null;
  updated?: string | null;
  normalization: string[];
  security?: unknown;
  collides_with?: string[];
  install_name?: string | null;
}

export interface SearchHit {
  id: string;
  kind: CatalogKind | string;
  name: string;
  category: string;
  description: string;
  source_id: string;
  repo?: string | null;
  license?: string | null;
  author?: string | null;
  tags: string[];
  origin_tool: string;
  stars?: number | null;
  updated?: string | null;
  file_count: number;
  size: number;
  collides: boolean;
  marketplace?: string | null;
  url?: string | null;
}

export interface SearchResult {
  total: number;
  offset: number;
  limit: number;
  items: SearchHit[];
}

export interface FacetValue {
  value: string;
  count: number;
}

export interface Facets {
  total: number;
  kind: FacetValue[];
  category: FacetValue[];
  source: FacetValue[];
  license: FacetValue[];
  origin_tool: FacetValue[];
}

export interface Filters {
  kinds?: string[];
  categories?: string[];
  sources?: string[];
  licenses?: string[];
  origin_tools?: string[];
  tags?: string[];
  marketplace?: string | null;
  hide_collisions?: boolean;
}

export interface CatalogMeta {
  generated?: string;
  total?: number;
  counts?: Record<string, number>;
  source?: { id: string; name: string; repo: string; commit: string; url: string; license: string; attribution: string };
  [k: string]: unknown;
}

export interface FileContent {
  encoding: "utf8" | "base64";
  content: string;
  sha256: string;
  size: number;
}

export interface ItemFiles {
  id: string;
  entry: string;
  verified: "sha256" | "git-blob" | "generated" | string;
  total_bytes: number;
  files: Record<string, FileContent>;
}

export interface RegistryPage {
  items: CatalogItem[];
  next_cursor?: string | null;
  count: number;
  from: string;
}

export interface MarketplaceInfo {
  repo: string;
  name: string;
  kind: string;
  description?: string | null;
  stars?: number | null;
  license?: string | null;
  updated?: string | null;
  commit?: string | null;
  website?: string | null;
  plugins: number;
  url?: string | null;
}

export interface UserSource {
  id: string;
  kind: "git" | "local" | "marketplace" | string;
  spec: string;
  repo?: string | null;
  path?: string | null;
  ref?: string | null;
  commit?: string | null;
  added: string;
  scanned?: string | null;
  items: number;
  excluded: number;
  error?: string | null;
}

export interface CollectionItem {
  item_id: string;
  position: number;
  targets: string[];
  scope?: string | null;
  added?: string | null;
}

export interface Collection {
  id: string;
  name: string;
  description?: string | null;
  position: number;
  targets: string[];
  scope?: string | null;
  created: string;
  updated: string;
  items: CollectionItem[];
}

export interface CollectionImport {
  collection: Collection;
  missing: string[];
  changed: string[];
}

// agentkit --------------------------------------------------------------

export type Compat =
  | { status: "native" }
  | { status: "converted" }
  | { status: "degraded"; lost: string[] }
  | { status: "unsupported"; reason: string };
export type CompatStatus = Compat["status"];

export interface TargetAdapter {
  id: string;
  name: string;
  tier: number;
  status: "active" | "maintenance" | "deprecated" | string;
  beta: boolean;
  homepage?: string;
  notes?: string;
  formats: Record<string, string>;
  reads_claude: Record<string, boolean>;
  mcp: unknown[];
  scopes?: Record<string, unknown>;
  [k: string]: unknown;
}

export interface ScopeInfo {
  scope: Scope;
  available: boolean;
  reason?: string | null;
  root?: string | null;
}

export interface DetectedTarget {
  id: string;
  name: string;
  tier: number;
  status: string;
  beta: boolean;
  installed: boolean;
  enabled_by_default: boolean;
  binaries: { name: string; path: string }[];
  version?: string | null;
  home_dirs: string[];
  vscode_extensions: string[];
  apps: string[];
  project_markers: string[];
  scopes: ScopeInfo[];
}

export interface Component {
  id: string;
  kind: string;
  name: string;
  category?: string | null;
  description?: string;
  source?: { id: string; repo?: string; commit?: string; path?: string; url?: string } | null;
  license?: string | null;
  author?: string | null;
  origin_tool?: string;
  compat?: Record<string, Compat>;
  files?: { path: string; text?: string; base64?: string }[];
  entry?: string;
  [k: string]: unknown;
}

export interface ComponentMeta {
  id: string;
  kind: string;
  name: string;
  category?: string | null;
  description: string;
  source?: { id: string; repo?: string; url?: string } | null;
  license?: string | null;
  sha256: string;
  origin_tool: string;
}

export type UnitStatus = "new" | "update" | "installed" | "unsupported" | "conflict" | "error";

export interface PlannedFile {
  target: string;
  component_id: string;
  path: string;
  action: "write" | "merge" | string;
  /** base64 (Write). */
  content?: string | null;
  executable: boolean;
  format?: string | null;
  losses?: string[];
  notes?: string[];
  commands?: string[];
  label: string;
  primary?: boolean;
}

export interface PlanUnit {
  unit_id: string;
  component: ComponentMeta;
  target: string;
  target_name: string;
  status: UnitStatus;
  compat: Compat;
  install_name: string;
  replaces?: string | null;
  files: PlannedFile[];
  losses: string[];
  notes: string[];
  commands: string[];
  error?: string | null;
}

export interface FilePlan {
  path: string;
  action: "create" | "replace" | "merge" | "unchanged" | "link" | string;
  /** Destino do symlink quando `action === "link"` (skills em `.agents/skills`). */
  link_to?: string | null;
  before_sha?: string | null;
  after_sha?: string | null;
  diff: string;
  targets: string[];
  components: string[];
  conflicts: string[];
  commands: string[];
  executable: boolean;
  owned_by_update: boolean;
}

export interface InstallPlan {
  id: string;
  created_at: string;
  scope: Scope;
  project_dir?: string | null;
  policy: Policy;
  units: PlanUnit[];
  files: FilePlan[];
  warnings: string[];
}

export interface WrittenFile {
  path: string;
  type: "created" | "merged" | "linked";
  /** Destino do link quando `type === "linked"`. */
  link_to?: string | null;
  sha256?: string;
  format?: string;
  created_file?: boolean;
}

export interface InstallRecord {
  install_id: string;
  component: ComponentMeta;
  target: string;
  scope: Scope;
  project_dir?: string | null;
  installed_name: string;
  compat: Compat;
  tx: string;
  installed_at: string;
  files: WrittenFile[];
  created_dirs: string[];
}

export interface ApplyReport {
  tx: string;
  installed: InstallRecord[];
  files_written: string[];
  unchanged: string[];
  skipped_units: string[];
  notes: string[];
}

export interface UninstallReport {
  tx: string;
  install_id: string;
  removed: string[];
  kept: string[];
  drift: string[];
}

export interface DriftItem {
  install_id: string;
  component: string;
  target: string;
  path: string;
  state: "missing" | "modified" | "piece_changed" | string;
  detail: string[];
}

export interface RestoreReport {
  tx: string;
  restored: string[];
  removed: string[];
}

export interface TxManifest {
  tx: string;
  started_at: string;
  kind: string;
  entries: unknown[];
}

// ------------------------------------------------------------------ tipos de UI

export const KIND_TABS: (CatalogKind | "all")[] = [
  "all",
  "agent",
  "command",
  "skill",
  "mcp",
  "hook",
  "setting",
  "statusline",
  "loop",
  "workflow",
  "plugin",
  "template",
  "mod",
];

/** Kinds that show up only when the index has them. */
export const EXTRA_KINDS: CatalogKind[] = ["rule", "sandbox"];

/** Glyph (static/icons) and gradient for each kind: chunky filled tiles. */
export const KIND_STYLE: Record<string, { glyph: string; from: string; to: string }> = {
  agent: { glyph: "robot", from: "#5E5CE6", to: "#3634A3" },
  command: { glyph: "terminal-window", from: "#48484A", to: "#1C1C1E" },
  skill: { glyph: "magic-wand", from: "#FF9F0A", to: "#E0600B" },
  mcp: { glyph: "plugs", from: "#30B0C7", to: "#0B7285" },
  hook: { glyph: "lightning", from: "#FFD60A", to: "#E09A00" },
  setting: { glyph: "sliders-horizontal", from: "#8E8E93", to: "#48484A" },
  statusline: { glyph: "gauge", from: "#34C759", to: "#1E7D34" },
  loop: { glyph: "arrows-clockwise", from: "#FF375F", to: "#B0103A" },
  workflow: { glyph: "list-checks", from: "#64D2FF", to: "#0A84FF" },
  mod: { glyph: "wrench", from: "#AC8E68", to: "#6B5436" },
  plugin: { glyph: "puzzle-piece", from: "#BF5AF2", to: "#7D2AA8" },
  rule: { glyph: "book-open-text", from: "#0A84FF", to: "#0040DD" },
  template: { glyph: "squares-four", from: "#FF6482", to: "#D7263D" },
  sandbox: { glyph: "cube", from: "#66D4CF", to: "#15807B" },
  all: { glyph: "stack", from: "#8E8E93", to: "#48484A" },
};

export function kindStyle(kind: string) {
  return KIND_STYLE[kind] ?? KIND_STYLE.all;
}

/** Kinds whose content is a folder (tree explorer instead of a single file). */
export const FOLDER_KINDS = new Set(["skill", "mod", "plugin", "template", "sandbox"]);
/** Kinds whose entry is JSON (tree viewer). */
export const JSON_KINDS = new Set(["mcp", "hook", "setting", "statusline"]);

// ------------------------------------------------------------------ catalog_*

export const catalogMeta = () => invoke<CatalogMeta>("catalog_meta");

export function catalogSearch(args: {
  query?: string;
  filters?: Filters;
  sort?: SortKey;
  offset?: number;
  limit?: number;
}): Promise<SearchResult> {
  return invoke<SearchResult>("catalog_search", {
    query: args.query ?? null,
    filters: args.filters ?? null,
    sort: args.sort ?? null,
    page: { offset: args.offset ?? 0, limit: args.limit ?? 48 },
  });
}

export const catalogFacets = (filters: Filters, query: string) =>
  invoke<Facets>("catalog_facets", { filters, query: query || null });

const itemCache = new Map<string, CatalogItem>();
export async function catalogItem(id: string): Promise<CatalogItem> {
  const hit = itemCache.get(id);
  if (hit) return hit;
  const item = await invoke<CatalogItem>("catalog_item", { id });
  itemCache.set(id, item);
  return item;
}

const filesCache = new Map<string, Promise<ItemFiles>>();
export function catalogItemFiles(id: string): Promise<ItemFiles> {
  let p = filesCache.get(id);
  if (!p) {
    p = invoke<ItemFiles>("catalog_item_files", { id });
    filesCache.set(id, p);
    p.catch(() => filesCache.delete(id));
  }
  return p;
}

export const registrySearch = (query: string, cursor: string | null, limit = 30) =>
  invoke<RegistryPage>("catalog_mcp_registry_search", { query: query || null, cursor, limit });

export const sourcesList = () => invoke<UserSource[]>("catalog_sources_list");
export const sourcesAdd = (spec: string, kind?: string | null) =>
  invoke<UserSource>("catalog_sources_add", { spec, kind: kind ?? null });
export const sourcesRescan = (id: string) => invoke<UserSource>("catalog_sources_rescan", { id });
export const sourcesRemove = (id: string) => invoke<void>("catalog_sources_remove", { id });
export const marketplaceLoad = (repo: string) =>
  invoke<{ info: MarketplaceInfo; items: CatalogItem[] }>("catalog_marketplace", { repo });
export const marketplacesList = () => invoke<MarketplaceInfo[]>("catalog_marketplaces");

export const collections = {
  list: () => invoke<Collection[]>("catalog_collections_list"),
  create: (name: string, description?: string | null, targets?: string[], scope?: string | null) =>
    invoke<Collection>("catalog_collections_create", {
      name,
      description: description ?? null,
      targets: targets ?? null,
      scope: scope ?? null,
    }),
  rename: (id: string, name: string, description?: string | null) =>
    invoke<Collection>("catalog_collections_rename", { id, name, description: description ?? null }),
  setDefaults: (id: string, targets: string[], scope?: string | null) =>
    invoke<Collection>("catalog_collections_set_defaults", { id, targets, scope: scope ?? null }),
  delete: (id: string) => invoke<void>("catalog_collections_delete", { id }),
  add: (id: string, itemId: string, targets?: string[], scope?: string | null) =>
    invoke<Collection>("catalog_collections_add", { id, itemId, targets: targets ?? null, scope: scope ?? null }),
  remove: (id: string, itemId: string) => invoke<Collection>("catalog_collections_remove", { id, itemId }),
  move: (id: string, itemId: string, to: number) => invoke<Collection>("catalog_collections_move", { id, itemId, to }),
  reorder: (id: string, to: number) => invoke<Collection[] | Collection>("catalog_collections_reorder", { id, to }),
  export: (id: string, path: string) => invoke<string>("catalog_collections_export", { id, path }),
  import: (path: string) => invoke<CollectionImport>("catalog_collections_import", { path }),
};

// ------------------------------------------------------------------ agentkit_*

let targetsPromise: Promise<TargetAdapter[]> | null = null;
export function agentkitTargets(): Promise<TargetAdapter[]> {
  if (!targetsPromise) {
    targetsPromise = invoke<TargetAdapter[]>("agentkit_targets");
    targetsPromise.catch(() => (targetsPromise = null));
  }
  return targetsPromise;
}

const detectCache = new Map<string, Promise<DetectedTarget[]>>();
export function agentkitDetect(projectDir: string | null, fresh = false): Promise<DetectedTarget[]> {
  const key = projectDir ?? "";
  let p = fresh ? undefined : detectCache.get(key);
  if (!p) {
    p = invoke<DetectedTarget[]>("agentkit_detect", { projectDir: projectDir || null });
    detectCache.set(key, p);
    p.catch(() => detectCache.delete(key));
  }
  return p;
}

export function agentkitParsePreview(args: {
  kind?: string;
  path?: string;
  entry?: string;
  files?: Record<string, string>;
  item?: CatalogItem;
}): Promise<Component> {
  return invoke<Component>("agentkit_parse_preview", {
    kind: args.kind ?? null,
    path: args.path ?? null,
    entry: args.entry ?? null,
    files: args.files ?? null,
    item: args.item ?? null,
  });
}

export interface PlanArgs {
  components?: Component[];
  catalogIds?: string[];
  targets: string[];
  scope?: Scope | null;
  projectDir?: string | null;
  policy?: Policy;
  secretValues?: Record<string, string> | null;
}

function rawPlan(a: PlanArgs): Promise<InstallPlan> {
  return invoke<InstallPlan>("agentkit_plan", {
    componentsJson: a.components && a.components.length ? a.components : null,
    catalogIds: a.catalogIds && a.catalogIds.length ? a.catalogIds : null,
    targets: a.targets,
    scope: a.scope ?? null,
    projectDir: a.projectDir || null,
    policy: a.policy ?? "rename",
    secretValues: a.secretValues ?? null,
  });
}

/**
 * Plans an install. Catalog ids go to the backend resolver; when the host has
 * no resolver wired (`AGENTKIT_NO_CATALOG`), each id is fetched, checked and
 * parsed here and sent as canonical components instead.
 */
export async function agentkitPlan(a: PlanArgs): Promise<InstallPlan> {
  try {
    return await rawPlan(a);
  } catch (e) {
    const msg = String(e);
    if (!msg.startsWith("AGENTKIT_NO_CATALOG") || !a.catalogIds?.length) throw e;
    const resolved = await Promise.all(a.catalogIds.map((id) => resolveComponent(id)));
    return rawPlan({ ...a, catalogIds: [], components: [...(a.components ?? []), ...resolved] });
  }
}

export const agentkitApply = (planId: string) => invoke<ApplyReport>("agentkit_apply", { planId });
export const agentkitUninstall = (installId: string, projectDir?: string | null, force = false) =>
  invoke<UninstallReport>("agentkit_uninstall", { installId, projectDir: projectDir || null, force });
export const agentkitInstalled = (projectDir?: string | null) =>
  invoke<InstallRecord[]>("agentkit_installed", { projectDir: projectDir || null });
export const agentkitDrift = (projectDir?: string | null) =>
  invoke<DriftItem[]>("agentkit_drift", { projectDir: projectDir || null });
export const agentkitRestore = (tx: string) => invoke<RestoreReport>("agentkit_restore", { tx });
export const agentkitTransactions = () => invoke<TxManifest[]>("agentkit_transactions");
export const agentkitImportInstalled = (target: string, scope?: Scope | null, projectDir?: string | null) =>
  invoke<Component[]>("agentkit_import_installed", { target, scope: scope ?? null, projectDir: projectDir || null });

/** Text files of an item (binary ones are skipped: the parser only needs text). */
export function textFiles(files: ItemFiles): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [p, f] of Object.entries(files.files)) if (f.encoding === "utf8") out[p] = f.content;
  return out;
}

const componentCache = new Map<string, Promise<Component>>();
/** Catalog id → canonical component (fetch + sha check + parse, with compat per tool). */
export function resolveComponent(id: string): Promise<Component> {
  let p = componentCache.get(id);
  if (!p) {
    p = (async () => {
      const [item, files] = await Promise.all([catalogItem(id), catalogItemFiles(id)]);
      const c = await agentkitParsePreview({ item, files: textFiles(files) });
      rememberCompat(id, c.compat);
      return c;
    })();
    componentCache.set(id, p);
    p.catch(() => componentCache.delete(id));
  }
  return p;
}

// ------------------------------------------------------------------ compatibilidade

/**
 * Compat of each card on each tool. Exact compat needs the item's files
 * (`agentkit_compat(itemId)` / `agentkit_parse_preview` download and parse
 * them), which is fine for one detail page and too heavy for a page of 48
 * cards: cards use an approximation from the target manifests that mirrors
 * `Registry::resolve` (kind + format + reads_claude), and exact values
 * learned on detail pages (`rememberCompat`) replace it.
 */
const compatById = new Map<string, Record<string, Compat>>();

export function rememberCompat(id: string, compat: Record<string, Compat> | undefined | null) {
  if (compat && Object.keys(compat).length) compatById.set(id, compat);
}

export async function compatFor(
  hits: { id: string; kind: string; origin_tool: string }[],
  targets: TargetAdapter[],
): Promise<Record<string, Record<string, Compat>>> {
  await loadKindMatrix();
  const out: Record<string, Record<string, Compat>> = {};
  for (const h of hits) {
    out[h.id] = compatById.get(h.id) ?? approxCompat(h.kind, h.origin_tool, targets);
  }
  return out;
}

/**
 * Compat per `(kind, tool)` measured by the backend on a minimal in-memory
 * sample of each kind (`agentkit_parse_preview`, no network, real
 * converters). Follows the converters as they are added; the manifest
 * approximation below covers kinds without a sample (loop, plugin…).
 */
const KIND_SAMPLES: Record<string, { entry: string; text: string }> = {
  agent: { entry: "sample.md", text: "---\nname: sample\ndescription: Sample agent\n---\nReview the code.\n" },
  command: { entry: "sample.md", text: "---\ndescription: Sample command\n---\nExplain $ARGUMENTS\n" },
  skill: { entry: "SKILL.md", text: "---\nname: sample\ndescription: Sample skill\n---\n# Sample\n" },
  mcp: { entry: "sample.json", text: '{"mcpServers":{"sample":{"command":"npx","args":["-y","sample-mcp"]}}}' },
  hook: {
    entry: "sample.json",
    text: '{"description":"Sample hook","hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"echo ok"}]}]}}',
  },
  setting: { entry: "sample.json", text: '{"description":"Sample setting","permissions":{"allow":["Read"]}}' },
  statusline: { entry: "sample.json", text: '{"description":"Sample statusline","statusLine":{"type":"command","command":"echo ok"}}' },
};
let kindMatrix: Record<string, Record<string, Compat>> = {};
let kindMatrixPromise: Promise<void> | null = null;

export function loadKindMatrix(): Promise<void> {
  if (!kindMatrixPromise) {
    kindMatrixPromise = (async () => {
      const out: Record<string, Record<string, Compat>> = {};
      await Promise.all(
        Object.entries(KIND_SAMPLES).map(async ([kind, s]) => {
          try {
            const c = await agentkitParsePreview({ kind, entry: s.entry, files: { [s.entry]: s.text } });
            if (c.compat) out[kind] = c.compat;
          } catch {
            /* this kind stays on the approximation */
          }
        }),
      );
      kindMatrix = out;
    })();
  }
  return kindMatrixPromise;
}

/** Exact compat of one item (backend `agentkit_compat`: fetch + parse + convert dry-run). */
export interface CompatRow {
  id: string;
  name: string;
  installed: boolean;
  beta: boolean;
  enabled_by_default: boolean;
  tier: number;
  compat: Compat;
}
export async function agentkitCompat(itemId: string, projectDir?: string | null): Promise<CompatRow[]> {
  const r = await invoke<{ targets: CompatRow[] }>("agentkit_compat", { itemId, projectDir: projectDir || null });
  rememberCompat(itemId, Object.fromEntries(r.targets.map((x) => [x.id, x.compat])));
  return r.targets;
}

const READS_KEY: Record<string, string> = {
  rule: "rules",
  agent: "agents",
  command: "commands",
  hook: "hooks",
  skill: "skills",
  plugin: "plugins",
  mod: "plugins",
  setting: "settings",
  statusline: "statusline",
};

/** One `(kind, tool)` pair, following the backend resolution order. */
export function kindCompat(kind: string, origin: string, t: TargetAdapter): Compat {
  const measured = kindMatrix[kind]?.[t.id];
  if (measured && !(kind === "mcp" && origin !== "claude")) return measured;
  if (kind === "mcp") {
    if (!t.mcp || t.mcp.length === 0) return { status: "unsupported", reason: `${t.name} has no MCP config file` };
    return t.id === origin ? { status: "native" } : { status: "converted" };
  }
  const fmt = t.formats?.[kind] || null;
  if (fmt === "claude") return t.id === "claude" ? { status: "native" } : { status: "converted" };
  if (kind === "skill" && fmt === "skill_dir") return { status: "native" };
  const rk = READS_KEY[kind];
  if (rk && (t.id === "claude" || t.reads_claude?.[rk])) return { status: "native" };
  return fmt
    ? { status: "unsupported", reason: `${kind} → ${t.name} format \`${fmt}\`: converter not available yet` }
    : { status: "unsupported", reason: `${t.name} has no ${kind}` };
}

export function approxCompat(kind: string, origin: string, targets: TargetAdapter[]): Record<string, Compat> {
  const out: Record<string, Compat> = {};
  for (const t of targets) out[t.id] = kindCompat(kind, origin || "claude", t);
  return out;
}

/** Kinds every selected tool can take (for the "works in" filter). */
export function kindsWorkingIn(toolIds: string[], targets: TargetAdapter[], kinds: string[]): string[] {
  const sel = targets.filter((t) => toolIds.includes(t.id));
  if (!sel.length) return kinds;
  return kinds.filter((k) => sel.every((t) => kindCompat(k, "claude", t).status !== "unsupported"));
}

export const COMPAT_ORDER: CompatStatus[] = ["native", "converted", "degraded", "unsupported"];

// ------------------------------------------------------------------ helpers puros

/** Two-letter monogram per tool id, unique across the tool table. */
const TOOL_MONO: Record<string, string> = {
  claude: "Cl",
  codex: "Cx",
  gemini: "Ge",
  qwen: "Qw",
  opencode: "Oc",
  kilo: "Ki",
  crush: "Cr",
  cursor: "Cu",
  copilot: "Gh",
  cline: "Cn",
  roo: "Ro",
  continue: "Ct",
  goose: "Go",
  zed: "Ze",
  droid: "Dr",
  kimi: "Km",
  pi: "Pi",
  amp: "Am",
  aider: "Ai",
  junie: "Ju",
  auggie: "Au",
  grok: "Gk",
  kiro: "Kr",
  devin: "Dv",
  windsurf: "Ws",
  warp: "Wa",
  trae: "Tr",
  qoder: "Qo",
  vibe: "Vi",
  letta: "Le",
  rovo: "Rv",
  antigravity: "Ag",
  codebuff: "Cb",
  openhands: "Oh",
  omniget: "Om",
};

export function toolMono(id: string, name: string): string {
  return TOOL_MONO[id] ?? initials(name);
}

export function initials(name: string): string {
  const parts = name.replace(/[^A-Za-z0-9 ]+/g, " ").trim().split(/\s+/);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  return (parts[0] ?? "?").slice(0, 2).replace(/^./, (c) => c.toUpperCase());
}

export function hue(id: string): number {
  let h = 0;
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) % 360;
  return h;
}

/** Route of the detail page for one id (each segment encoded, `/` kept). */
export function itemHref(id: string): string {
  return "/llm/catalog/" + id.split("/").map(encodeURIComponent).join("/");
}

export function compactNumber(n: number | null | undefined): string {
  if (n == null) return "";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1000) return (n / 1000).toFixed(1).replace(/\.0$/, "") + "k";
  return String(n);
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** ~4 chars per token, the same approximation the original site used. */
export function approxTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

export function decodeBase64Text(b64: string): string {
  try {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch {
    return "";
  }
}

/** Splits `---\nyaml\n---\nbody`; no frontmatter → `["", text]`. */
export function splitFrontmatter(text: string): [string, string] {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  if (!m) return ["", text];
  return [m[1], text.slice(m[0].length)];
}

export interface Heading {
  level: number;
  text: string;
  line: number;
}

/** ATX headings outside code fences. */
export function headingsOf(md: string): Heading[] {
  const out: Heading[] = [];
  let fence = false;
  md.split("\n").forEach((l, i) => {
    if (/^\s*(```|~~~)/.test(l)) fence = !fence;
    if (fence) return;
    const m = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(l);
    if (m) out.push({ level: m[1].length, text: m[2].replace(/[`*_]/g, ""), line: i });
  });
  return out;
}

export interface Slide {
  title: string;
  body: string;
  tokens: number;
}

/**
 * SKILL.md → slides: a title slide (frontmatter name/description) and one
 * slide per `#`/`##` section; `###` and deeper stay inside their section.
 */
export function slidesOf(text: string, fallbackTitle: string): Slide[] {
  const [fm, body] = splitFrontmatter(text);
  const name = /^name:\s*(.+)$/m.exec(fm)?.[1]?.replace(/^["']|["']$/g, "") ?? fallbackTitle;
  const desc = /^description:\s*(.+)$/m.exec(fm)?.[1]?.replace(/^["']|["']$/g, "") ?? "";
  const slides: Slide[] = [];
  const intro: string[] = [];
  let cur: { title: string; lines: string[] } | null = null;
  let fence = false;
  for (const l of body.split("\n")) {
    if (/^\s*(```|~~~)/.test(l)) fence = !fence;
    const m = !fence ? /^(#{1,2})\s+(.+)$/.exec(l) : null;
    if (m) {
      if (cur) slides.push({ title: cur.title, body: cur.lines.join("\n").trim(), tokens: 0 });
      cur = { title: m[2].trim(), lines: [] };
    } else if (cur) cur.lines.push(l);
    else intro.push(l);
  }
  if (cur) slides.push({ title: cur.title, body: cur.lines.join("\n").trim(), tokens: 0 });
  const first: Slide = { title: name, body: [desc, intro.join("\n").trim()].filter(Boolean).join("\n\n"), tokens: 0 };
  const all = [first, ...slides.filter((s) => s.title || s.body)];
  for (const s of all) s.tokens = approxTokens(`${s.title}\n${s.body}`);
  return all;
}

export interface TreeNode {
  name: string;
  path: string;
  dir: boolean;
  children: TreeNode[];
  size?: number;
}

/** Flat relative paths → sorted tree (folders first). */
export function buildTree(paths: { path: string; size?: number }[]): TreeNode[] {
  const root: TreeNode = { name: "", path: "", dir: true, children: [] };
  for (const f of paths) {
    const parts = f.path.split("/");
    let node = root;
    parts.forEach((part, i) => {
      const leaf = i === parts.length - 1;
      const p = parts.slice(0, i + 1).join("/");
      let child = node.children.find((c) => c.name === part && c.dir === !leaf);
      if (!child) {
        child = { name: part, path: p, dir: !leaf, children: [], size: leaf ? f.size : undefined };
        node.children.push(child);
      }
      node = child;
    });
  }
  const sort = (n: TreeNode) => {
    n.children.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1));
    n.children.forEach(sort);
  };
  sort(root);
  return root.children;
}

export function languageOfPath(p: string): string {
  const ext = p.split(".").pop()?.toLowerCase() ?? "";
  return (
    {
      md: "markdown",
      mdx: "markdown",
      json: "json",
      jsonc: "json",
      toml: "toml",
      yaml: "yaml",
      yml: "yaml",
      py: "python",
      sh: "bash",
      js: "javascript",
      mjs: "javascript",
      ts: "typescript",
    } as Record<string, string>
  )[ext] ?? "text";
}

/** FilePlan → the diff viewer's FileDiff shape. */
export function filePlanToDiff(f: FilePlan) {
  let additions = 0;
  let deletions = 0;
  for (const l of f.diff.split("\n")) {
    if (l.startsWith("+++") || l.startsWith("---")) continue;
    if (l.startsWith("+")) additions++;
    else if (l.startsWith("-")) deletions++;
  }
  return {
    path: f.path,
    status: (f.action === "create" ? "added" : "modified") as "added" | "modified",
    additions,
    deletions,
    binary: false,
    patch: f.diff,
    truncated: false,
  };
}

/** CLI line equivalent to the stack (for scripts and README snippets). */
export function cliCommand(args: {
  ids: string[];
  targets: string[];
  scope: Scope;
  projectDir?: string | null;
  policy: Policy;
}): string {
  const q = (s: string) => (/^[A-Za-z0-9_./:@-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);
  const parts = ["omniget", "agentkit", "install", ...args.ids.map(q)];
  if (args.targets.length) parts.push("--target", args.targets.join(","));
  parts.push("--scope", args.scope);
  if (args.scope !== "global" && args.projectDir) parts.push("--project", q(args.projectDir));
  if (args.policy !== "rename") parts.push("--policy", args.policy);
  return parts.join(" ");
}

export function errText(e: unknown): string {
  const s = String(e);
  const m = /^([A-Z_]+):\s*(.*)$/s.exec(s);
  return m ? m[2] : s;
}

export function errCode(e: unknown): string {
  return /^([A-Z_]+):/.exec(String(e))?.[1] ?? "";
}

// ------------------------------------------------------------------ projetos recentes

const RECENT_KEY = "omniget.central.catalog.projects";

export function recentProjects(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

export function rememberProject(dir: string) {
  try {
    const list = [dir, ...recentProjects().filter((d) => d !== dir)].slice(0, 8);
    localStorage.setItem(RECENT_KEY, JSON.stringify(list));
  } catch {
    /* storage off: nothing to remember */
  }
}

/** The folder of the LLM workspace, when there is one. */
export async function workspaceFolder(): Promise<string | null> {
  try {
    const ws = await invoke<{ path: string | null }>("llm_workspace_get", { conversationId: null });
    return ws?.path ?? null;
  } catch {
    return null;
  }
}

export function baseName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** Full item → the lighter search-hit shape the cards take. */
export function toHit(i: CatalogItem): SearchHit {
  return {
    id: i.id,
    kind: i.kind,
    name: i.name,
    category: i.category,
    description: i.description,
    source_id: i.source.id,
    repo: i.source.repo ?? null,
    license: i.license ?? null,
    author: i.author ?? null,
    tags: i.tags ?? [],
    origin_tool: i.origin_tool,
    stars: i.stars ?? null,
    updated: i.updated ?? null,
    file_count: i.files?.length ?? 0,
    size: (i.files ?? []).reduce((a, f) => a + f.size, 0),
    collides: !!i.collides_with?.length,
    marketplace: i.source.marketplace ?? null,
    url: i.source.url ?? null,
  };
}

export function sourceLabel(id: string): string {
  if (id === "cct") return "claude-code-templates";
  if (id === "mcp-registry") return "MCP Registry";
  return id;
}

const PLURAL: Record<string, string> = {
  agent: "agents",
  command: "commands",
  skill: "skills",
  mcp: "mcps",
  hook: "hooks",
  setting: "settings",
  statusline: "statuslines",
  loop: "loops",
  workflow: "workflows",
  mod: "mods",
  plugin: "plugins",
  rule: "rules",
  template: "templates",
  sandbox: "sandbox",
};

/** `agent:cat/name` (loop/stack reference) → catalog id in the same source. */
export function refToId(ref: string, sourceId: string): string {
  if (ref.includes(":") && ref.split(":")[1].includes("/") && PLURAL[ref.split(":")[0]]) {
    const [kind, rest] = [ref.slice(0, ref.indexOf(":")), ref.slice(ref.indexOf(":") + 1)];
    return `${sourceId}:${PLURAL[kind]}/${rest}`;
  }
  return ref;
}

export function refKind(ref: string): string {
  const k = ref.slice(0, ref.indexOf(":"));
  return PLURAL[k] ? k : "all";
}
