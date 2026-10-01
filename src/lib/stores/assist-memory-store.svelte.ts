import { invoke } from "@tauri-apps/api/core";

export type MemoryKind = "declared" | "observation" | "inference" | "temporary" | "procedural";
type MemoryStatus = "active" | "superseded" | "retracted" | "expired";

type MemoryScope =
  | { kind: "user" }
  | { kind: "bot"; bot: string }
  | { kind: "room"; conversation: string };

interface MemorySource {
  kind: string;
  id?: string | null;
  conversation?: string | null;
  author: string;
  at_ms: number;
}

export interface MemoryItem {
  id: string;
  scope: MemoryScope;
  scope_key: string;
  category: MemoryKind;
  subject?: string | null;
  content: string;
  data?: Record<string, unknown> | null;
  source: MemorySource;
  source_line: string;
  confidence: number;
  evidence?: string | null;
  valid_until?: number | null;
  version: number;
  chain: string;
  supersedes?: string | null;
  status: MemoryStatus;
  created_ms: number;
  updated_ms: number;
}

interface MemoryScopeSummary {
  scope: MemoryScope;
  scope_key: string;
  active: number;
  candidates: number;
  inactive: number;
}

export interface ImportReport {
  imported: number;
  duplicates: number;
  skipped_forgotten: number;
  resurrected: number;
  conflicts_resolved: number;
  tombstones_added: number;
  rejected: string[];
}

interface BackupsInfo {
  count: number;
  newest_ms: number | null;
}

export const KIND_ORDER: MemoryKind[] = ["declared", "temporary", "inference", "observation", "procedural"];

export function scopeKey(scope: MemoryScope): string {
  if (scope.kind === "user") return "user";
  if (scope.kind === "bot") return `bot:${scope.bot}`;
  return `room:${scope.conversation}`;
}

export function botScopes(botId: string): string[] {
  return ["user", `bot:${botId}`];
}

export function groupByKind(items: MemoryItem[]): { kind: MemoryKind; items: MemoryItem[] }[] {
  return KIND_ORDER.map((kind) => ({
    kind,
    items: items.filter((i) => i.category === kind && i.status === "active"),
  })).filter((g) => g.items.length > 0);
}

function memoryErrorKey(err: unknown): string {
  const s = String((err as { message?: string })?.message ?? err ?? "");
  const code = s.split(":")[0]?.trim();
  switch (code) {
    case "ERR_MEMORY_NOT_FOUND":
      return "assist.memory.err_not_found";
    case "ERR_MEMORY_INVALID":
      return "assist.memory.err_invalid";
    case "ERR_MEMORY_FORGOTTEN":
      return "assist.memory.err_forgotten";
    case "ERR_MEMORY_INACTIVE":
      return "assist.memory.err_inactive";
    case "ERR_MEMORY_IMPORT":
      return "assist.memory.err_import";
    case "ERR_ASSIST_SCOPE":
      return "assist.memory.err_scope";
    default:
      return "assist.memory.err_generic";
  }
}

export function authorBot(item: MemoryItem): string | null {
  return item.source.author.startsWith("bot:") ? item.source.author.slice(4) : null;
}

const state = $state({
  byScope: {} as Record<string, MemoryItem[]>,
  scopes: [] as MemoryScopeSummary[],
  loading: false,
  errorKey: null as string | null,
  backups: null as BackupsInfo | null,
});

export function getScopeItems(key: string): MemoryItem[] {
  return state.byScope[key] ?? [];
}

export function getScopes(): MemoryScopeSummary[] {
  return state.scopes;
}

export function isMemoryLoading(): boolean {
  return state.loading;
}

export function getMemoryErrorKey(): string | null {
  return state.errorKey;
}

export function getBackups(): BackupsInfo | null {
  return state.backups;
}

export async function loadScopes(keys: string[], includeInactive = false): Promise<void> {
  state.loading = true;
  state.errorKey = null;
  try {
    const results = await Promise.all(
      keys.map((scope) => invoke<MemoryItem[]>("assist_memory_list", { scope, includeInactive })),
    );
    const next = { ...state.byScope };
    keys.forEach((k, i) => {
      next[k] = results[i] ?? [];
    });
    state.byScope = next;
  } catch (e) {
    state.errorKey = memoryErrorKey(e);
  } finally {
    state.loading = false;
  }
}

export async function loadAllScopes(includeInactive = false): Promise<void> {
  state.loading = true;
  state.errorKey = null;
  try {
    state.scopes = (await invoke<MemoryScopeSummary[]>("assist_memory_scopes")) ?? [];
  } catch (e) {
    state.errorKey = memoryErrorKey(e);
    state.loading = false;
    return;
  }
  const keys = state.scopes.map((s) => s.scope_key);
  if (!keys.includes("user")) keys.unshift("user");
  await loadScopes(keys, includeInactive);
}

async function refresh(keys: string[]): Promise<void> {
  await loadScopes(keys.filter((k, i) => keys.indexOf(k) === i));
}

type Outcome<T> = { ok: true; value: T } | { ok: false; errorKey: string };

async function run<T>(cmd: string, args: Record<string, unknown>, touched: string[]): Promise<Outcome<T>> {
  try {
    const value = await invoke<T>(cmd, args);
    await refresh(touched);
    return { ok: true, value };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export function createMemory(
  scope: string,
  kind: MemoryKind,
  content: string,
  subject?: string,
): Promise<Outcome<MemoryItem>> {
  return run("assist_memory_create", { input: { scope, kind, content, subject: subject || null } }, [scope]);
}

export function correctMemory(item: MemoryItem, content: string, kind?: MemoryKind): Promise<Outcome<MemoryItem>> {
  return run("assist_memory_correct", { input: { id: item.id, content, kind: kind ?? null } }, [item.scope_key]);
}

export function confirmMemory(item: MemoryItem): Promise<Outcome<MemoryItem>> {
  return run("assist_memory_confirm", { id: item.id }, [item.scope_key]);
}

export function retractMemory(item: MemoryItem): Promise<Outcome<MemoryItem>> {
  return run("assist_memory_retract", { id: item.id }, [item.scope_key]);
}

export function forgetMemory(item: MemoryItem): Promise<Outcome<number>> {
  return run("assist_memory_forget", { id: item.id }, [item.scope_key]);
}

export async function memoryHistory(id: string): Promise<Outcome<MemoryItem[]>> {
  try {
    return { ok: true, value: await invoke<MemoryItem[]>("assist_memory_history", { id }) };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export async function exportMemory(format: "json" | "markdown"): Promise<Outcome<string> | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  const ext = format === "json" ? "json" : "md";
  const path = await save({
    defaultPath: `omniget-memory.${ext}`,
    filters: [{ name: format === "json" ? "JSON" : "Markdown", extensions: [ext] }],
  });
  if (!path) return null;
  try {
    const out = await invoke<{ path: string | null }>("assist_memory_export", { format, path });
    return { ok: true, value: out.path ?? path };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export async function importMemory(allowResurrect: boolean): Promise<Outcome<ImportReport> | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const path = await open({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
  if (!path || Array.isArray(path)) return null;
  try {
    const value = await invoke<ImportReport>("assist_memory_import", { path, allowResurrect });
    await loadAllScopes();
    return { ok: true, value };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export async function loadBackups(): Promise<void> {
  try {
    state.backups = await invoke<BackupsInfo>("assist_memory_backups");
  } catch {
    state.backups = null;
  }
}

export async function deleteBackups(): Promise<Outcome<number>> {
  try {
    const value = await invoke<number>("assist_memory_delete_backups");
    await loadBackups();
    return { ok: true, value };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export async function reindexMemory(): Promise<Outcome<number>> {
  try {
    return { ok: true, value: await invoke<number>("assist_memory_reindex") };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export interface MemoryStats {
  active: number;
  with_vector: number;
  entities: number;
  notes: number;
  links: number;
  retains_today: number;
  facts_today: number;
  last_retain_ms?: number | null;
  last_retain_error?: string | null;
  semantic_ready: boolean;
  auto_retain: boolean;
  consolidate: boolean;
}

export interface MemoryEntity {
  id: string;
  name: string;
  kind: string;
  mentions: number;
  last_ms: number;
  note?: string | null;
}

type RecallArm = "semantic" | "keyword" | "graph" | "temporal";

interface RecallWhy {
  arms: RecallArm[];
  semantic?: number | null;
  keyword_rank?: number | null;
  graph?: number | null;
  temporal?: number | null;
  rrf: number;
  proofs: number;
  occurred_start?: number | null;
  occurred_end?: number | null;
}

interface RecallHit extends MemoryItem {
  score: number;
  why: RecallWhy;
}

export interface RecallResult {
  hits: RecallHit[];
  entities: string[];
  window?: [number, number] | null;
  semantic: boolean;
}

interface RetainReport {
  extracted: number;
  created: string[];
  reinforced: string[];
  skipped: number;
  duplicate: boolean;
}

async function attempt<T>(f: () => Promise<T>): Promise<Outcome<T>> {
  try {
    return { ok: true, value: await f() };
  } catch (e) {
    return { ok: false, errorKey: memoryErrorKey(e) };
  }
}

export async function memoryStats(): Promise<Outcome<MemoryStats>> {
  return attempt(() => invoke<MemoryStats>("assist_memory_stats"));
}

export async function memoryEntities(limit = 60): Promise<Outcome<MemoryEntity[]>> {
  return attempt(() => invoke<MemoryEntity[]>("assist_memory_entities", { limit }));
}

export async function searchMemory(query: string, limit = 20): Promise<Outcome<RecallResult>> {
  return attempt(() => invoke<RecallResult>("assist_memory_search", { query, limit }));
}

export async function setMemorySettings(input: {
  auto_retain?: boolean;
  consolidate?: boolean;
}): Promise<Outcome<MemoryStats>> {
  return attempt(() => invoke<MemoryStats>("assist_memory_set_settings", { input }));
}

export async function prepareSemanticMemory(): Promise<Outcome<number>> {
  return attempt(() => invoke<number>("assist_memory_prepare_semantic"));
}

export async function retainText(text: string, context?: string): Promise<Outcome<RetainReport>> {
  const out = await attempt(() => invoke<RetainReport>("assist_memory_retain_text", { text, context }));
  if (out.ok) await loadAllScopes(false);
  return out;
}
