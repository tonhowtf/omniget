import { invoke } from "@tauri-apps/api/core";

type CatalogKind =
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
type Scope = "project" | "global" | "local" | "managed";
type Policy = "rename" | "skip" | "overwrite";

interface ItemSource {
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

interface FileContent {
  encoding: "utf8" | "base64";
  content: string;
  sha256: string;
  size: number;
}

interface ItemFiles {
  id: string;
  entry: string;
  verified: "sha256" | "git-blob" | "generated" | string;
  total_bytes: number;
  files: Record<string, FileContent>;
}

type Compat =
  | { status: "native" }
  | { status: "converted" }
  | { status: "degraded"; lost: string[] }
  | { status: "unsupported"; reason: string };

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

interface ComponentMeta {
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

type UnitStatus = "new" | "update" | "installed" | "unsupported" | "conflict" | "error";

interface PlannedFile {
  target: string;
  component_id: string;
  path: string;
  action: "write" | "merge" | string;
  content?: string | null;
  executable: boolean;
  format?: string | null;
  losses?: string[];
  notes?: string[];
  commands?: string[];
  label: string;
  primary?: boolean;
}

interface PlanUnit {
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

interface FilePlan {
  path: string;
  action: "create" | "replace" | "merge" | "unchanged" | "link" | string;
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

interface WrittenFile {
  path: string;
  type: "created" | "merged" | "linked";
  link_to?: string | null;
  sha256?: string;
  format?: string;
  created_file?: boolean;
}

interface InstallRecord {
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

const itemCache = new Map<string, CatalogItem>();
async function catalogItem(id: string): Promise<CatalogItem> {
  const hit = itemCache.get(id);
  if (hit) return hit;
  const item = await invoke<CatalogItem>("catalog_item", { id });
  itemCache.set(id, item);
  return item;
}

const filesCache = new Map<string, Promise<ItemFiles>>();
function catalogItemFiles(id: string): Promise<ItemFiles> {
  let p = filesCache.get(id);
  if (!p) {
    p = invoke<ItemFiles>("catalog_item_files", { id });
    filesCache.set(id, p);
    p.catch(() => filesCache.delete(id));
  }
  return p;
}

let targetsPromise: Promise<TargetAdapter[]> | null = null;
export function agentkitTargets(): Promise<TargetAdapter[]> {
  if (!targetsPromise) {
    targetsPromise = invoke<TargetAdapter[]>("agentkit_targets");
    targetsPromise.catch(() => (targetsPromise = null));
  }
  return targetsPromise;
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

interface PlanArgs {
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

function textFiles(files: ItemFiles): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [p, f] of Object.entries(files.files)) if (f.encoding === "utf8") out[p] = f.content;
  return out;
}

const componentCache = new Map<string, Promise<Component>>();
function resolveComponent(id: string): Promise<Component> {
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

const compatById = new Map<string, Record<string, Compat>>();

function rememberCompat(id: string, compat: Record<string, Compat> | undefined | null) {
  if (compat && Object.keys(compat).length) compatById.set(id, compat);
}
