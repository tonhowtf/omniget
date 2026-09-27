// Invokes tipados dos comandos `sessions_*` da Central (/llm/usage,
// /llm/sessions, /llm/retro): índice de sessões de todas as ferramentas de
// código com LLM, busca, análise por sessão, time, retrospectiva, exportar e
// importar. Os structs Rust chegam em snake_case; os argumentos de topo dos
// comandos vão em camelCase (conversão do Tauri). Erros: "CODE: mensagem".

import { invoke } from "@tauri-apps/api/core";

// ---- tipos (espelham omniget_core::core::sessions) ----

export type TokenUsage = {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  reasoning: number;
};

export type Role = "user" | "assistant" | "system" | "tool";
export type ToolStatus = "ok" | "error" | "pending";

export type ToolCall = {
  id: string;
  name_canonical: string;
  name_raw: string;
  input: unknown;
  result: string | null;
  status: ToolStatus;
  ms: number | null;
  subagent: string | null;
};

export type Turn = {
  role: Role;
  ts: string;
  text: string;
  tool_calls: ToolCall[];
  usage: TokenUsage;
  cost_usd: number | null;
  model: string | null;
  message_id: string | null;
};

export type SessionMeta = {
  tool: string;
  account: string | null;
  id: string;
  title: string | null;
  project_path: string | null;
  git_branch: string | null;
  started: string | null;
  ended: string | null;
  models: string[];
  parent_session: string | null;
  turn_count: number;
  usage: TokenUsage;
  cost_usd: number;
  source: string;
  mtime_ms: number;
};

export type ListFilter = {
  tool?: string | null;
  account?: string | null;
  project?: string | null;
  from?: string | null;
  to?: string | null;
  query?: string | null;
  include_subagents?: boolean;
  parent?: string | null;
  sort?: "recent" | "tokens" | "cost" | "oldest" | null;
  limit?: number | null;
  offset?: number | null;
  id?: string | null;
  /** Só é honrado quando o índice aceita filtro por modelo (ver handoff). */
  model?: string | null;
};

export type ListPage = { total: number; sessions: SessionMeta[] };

export type SessionPage = {
  meta: SessionMeta;
  turns: Turn[];
  first_index: number;
  page: number;
  pages: number;
  total: number;
  has_older: boolean;
  resume_command: string | null;
};

export type SearchQuery = {
  text: string;
  tool?: string | null;
  account?: string | null;
  project?: string | null;
  from?: string | null;
  to?: string | null;
  include_subagents?: boolean;
  limit?: number | null;
  per_session?: number | null;
  in_tools?: boolean | null;
};

export type HitField = "title" | "project" | "text" | "tool_name" | "tool_input" | "tool_result";

export type Hit = {
  tool: string;
  session_id: string;
  title: string | null;
  project_path: string | null;
  ended: string | null;
  turn: number | null;
  role: Role | null;
  ts: string | null;
  field: HitField;
  snippet: string;
  match_start: number;
  match_len: number;
  match_count: number;
};

export type SearchResult = {
  hits: Hit[];
  sessions_scanned: number;
  sessions_matched: number;
  truncated: boolean;
};

export type CostSource = "recorded" | "computed" | "mixed" | "unknown";

export type SessionAnalysis = {
  meta: SessionMeta;
  duration_ms: number;
  time: {
    agent_ms: number;
    think_ms: number;
    away_ms: number;
    agent_pct: number;
    think_pct: number;
    exchanges: number;
  };
  cache: { read: number; write: number; efficiency: number; hit_ratio: number };
  usage: TokenUsage;
  cost: {
    cost_usd: number;
    recorded_usd: number;
    computed_usd: number;
    unpriced_tokens: number;
    unpriced_models: string[];
    source: CostSource;
  };
  models: { model: string; responses: number; pct: number; tokens: number; cost_usd: number | null }[];
  tools: { name: string; raw: string[]; count: number; errors: number; avg_ms: number | null; total_ms: number }[];
  timeline: { turn: number; ts: string; name: string; raw: string; status: ToolStatus }[];
  components: {
    agents: [string, number][];
    slash_commands: [string, number][];
    skills: [string, number][];
    mcp_servers: [string, number][];
  };
  counts: { turns: number; user: number; assistant: number; tool_calls: number; tool_errors: number };
  max_context: number;
  tips: { code: string; severity: "info" | "warn"; value: number; message: string }[];
};

export type GroupBy = "day" | "week" | "month" | "model" | "tool" | "project" | "account" | "hour" | "session";

export type UsageBucket = {
  key: string;
  usage: TokenUsage;
  total_tokens: number;
  cost_usd: number;
  recorded_usd: number;
  unpriced_tokens: number;
  responses: number;
  sessions: number;
};

export type UsageReport = {
  group_by: GroupBy;
  rows: UsageBucket[];
  totals: UsageBucket;
  unpriced_models: string[];
};

export type HeatDay = {
  date: string;
  sessions: number;
  messages: number;
  user_messages: number;
  tokens: number;
  tool_calls: number;
  level: number;
};

export type Heatmap = {
  from: string;
  to: string;
  days: HeatDay[];
  stats: {
    active_days: number;
    current_streak: number;
    longest_streak: number;
    longest_streak_start: string | null;
    longest_streak_end: string | null;
    total_sessions: number;
    total_messages: number;
    total_tokens: number;
    total_tool_calls: number;
    peak_hour: number | null;
    hours: number[];
    peak_weekday: number | null;
    weekdays: number[];
    busiest_day: string | null;
  };
};

export type AgentsReport = {
  agents: {
    name: string;
    invocations: number;
    sessions: number;
    tools: string[];
    first: string | null;
    last: string | null;
    hourly: number[];
    daily: [string, number][];
    prompts: string[];
  }[];
  patterns: { pattern: string; count: number }[];
  timeline: { ts: string; tool: string; session_id: string; agent: string }[];
  popular_hours: number[];
  by_day: [string, number][];
  by_tool: [string, number][];
  subagent_sessions: [string, number][];
  total_invocations: number;
  agent_types: number;
  top_agent: string | null;
  avg_invocations_per_type: number;
  adoption_rate: number;
};

export type TeamNode = {
  id: string;
  name: string;
  kind: "lead" | "subagent" | "teammate";
  session_id: string | null;
  spawned_by: string | null;
  spawn_call_id: string | null;
  agent_type: string | null;
  description: string | null;
  started: string | null;
  ended: string | null;
  turns: number;
  tool_calls: number;
  messages_sent: number;
  messages_received: number;
  tools_used: [string, number][];
  usage: TokenUsage;
};

export type TeamGraph = {
  tool: string;
  session_id: string;
  is_team: boolean;
  nodes: TeamNode[];
  edges: { from: string; to: string; kind: "spawn" | "message"; count: number; samples: string[] }[];
  tasks: {
    id: string;
    subject: string | null;
    description: string | null;
    status: string | null;
    owner: string | null;
    created_by: string | null;
    history: [string, string][];
  }[];
  messages: { ts: string; from: string; to: string; text: string }[];
};

export type Ranked = { name: string; count: number; tokens: number; pct: number };

export type Milestone = { date: string; kind: string; label: string; value: number };

export type Retro = {
  from: string;
  to: string;
  totals: {
    sessions: number;
    messages: number;
    user_messages: number;
    tool_calls: number;
    usage: TokenUsage;
    total_tokens: number;
    cost_usd: number;
    active_days: number;
    projects: number;
  };
  top_models: Ranked[];
  top_tools: Ranked[];
  top_projects: Ranked[];
  top_agents: Ranked[];
  top_slash_commands: Ranked[];
  top_skills: Ranked[];
  top_mcp: Ranked[];
  by_tool: Ranked[];
  most_productive_day: HeatDay | null;
  peak_hour: number | null;
  busiest_weekday: number | null;
  current_streak: number;
  longest_streak: number;
  heatmap: HeatDay[];
  milestones: Milestone[];
};

export type ExportFormat = "json" | "markdown" | "context";

export type ExportOut = {
  filename: string;
  mime: string;
  content: string;
  turns_included: number;
  turns_total: number;
};

export type ImportOutcome = {
  source_tool: string;
  target_tool: string;
  session_id: string | null;
  written: string | null;
  resume_command: string | null;
  context: ExportOut | null;
};

export type SourceInfo = {
  tool: string;
  roots: { path: string; exists: boolean }[];
  found: boolean;
  beta: boolean;
  sessions: number;
  last_activity: string | null;
};

export type ActiveState = "working" | "waiting" | "idle";

export type ActiveSession = {
  meta: SessionMeta;
  state: ActiveState;
  idle_secs: number;
  pending_tool: boolean;
};

export type RefreshStats = {
  sources: number;
  sessions_seen: number;
  full_reads: number;
  tail_reads: number;
  removed: number;
  elapsed_ms: number;
};

// ---- comandos ----

/** Tira campos vazios: o Rust trata `""` como filtro literal em alguns lugares. */
function clean<T extends Record<string, unknown>>(f: T): T {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(f)) {
    if (v === undefined || v === null || v === "") continue;
    out[k] = v;
  }
  return out as T;
}

export const sessionsList = (filter: ListFilter = {}) =>
  invoke<ListPage>("sessions_list", { filter: clean(filter) });

export const sessionsGet = (tool: string, id: string, page?: number, pageSize?: number) =>
  invoke<SessionPage>("sessions_get", { tool, id, page, pageSize });

export const sessionsSearch = (query: SearchQuery) =>
  invoke<SearchResult>("sessions_search", { query: clean(query) });

export const sessionsSearchIn = (tool: string, id: string, query: string) =>
  invoke<Hit[]>("sessions_search_in", { tool, id, query });

export const sessionsAnalysis = (tool: string, id: string) =>
  invoke<SessionAnalysis>("sessions_analysis", { tool, id });

export const sessionsUsage = (range: ListFilter, groupBy: GroupBy) =>
  invoke<UsageReport>("sessions_usage", { range: clean(range), groupBy });

export const sessionsHeatmap = (range: ListFilter = {}, tool?: string | null) =>
  invoke<Heatmap>("sessions_heatmap", { range: clean(range), tool: tool || null });

export const sessionsAgents = (range: ListFilter = {}) =>
  invoke<AgentsReport>("sessions_agents", { range: clean(range) });

export const sessionsTeam = (tool: string, id: string) =>
  invoke<TeamGraph>("sessions_team", { tool, id });

export const sessionsRetro = (from?: string | null, to?: string | null, tool?: string | null) =>
  invoke<Retro>("sessions_retro", { from: from || null, to: to || null, tool: tool || null });

export const sessionsExport = (tool: string, id: string, format: ExportFormat, target?: string | null) =>
  invoke<ExportOut>("sessions_export", { tool, id, format, target: target || null });

export const sessionsImport = (path: string, target?: string | null) =>
  invoke<ImportOutcome>("sessions_import", { path, target: target || null });

export const sessionsResumeCommand = (tool: string, id: string) =>
  invoke<string | null>("sessions_resume_command", { tool, id });

export const sessionsSources = () => invoke<SourceInfo[]>("sessions_sources");

export const sessionsActive = (windowSecs?: number) =>
  invoke<ActiveSession[]>("sessions_active", { windowSecs });

export const sessionsRefresh = (full = false) => invoke<RefreshStats>("sessions_refresh", { full });

/**
 * "Abrir como thread": o comando é da rodada das threads e pode não existir
 * ainda. Devolve `null` quando o comando não está registrado.
 */
export async function importAsThread(tool: string, id: string): Promise<unknown | null> {
  try {
    return await invoke("threads_import_external", { tool, id, sessionId: id });
  } catch (e) {
    const msg = errText(e);
    if (/not found|unknown command|not allowed|command .* not/i.test(msg)) return null;
    throw e;
  }
}

/** Grava texto num caminho escolhido pelo usuário (diálogo de salvar). */
export async function saveTextAs(filename: string, content: string, ext: string): Promise<string | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  const dest = await save({ defaultPath: filename, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (typeof dest !== "string") return null;
  await invoke("tool_x_write_text", { dest, content });
  return dest;
}

export async function pickJsonFile(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] });
  return typeof r === "string" ? r : null;
}

export async function copyText(text: string): Promise<void> {
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  } catch {
    await navigator.clipboard.writeText(text);
  }
}

// ---- nomes e formatação ----

export const TOOL_NAMES: Record<string, string> = {
  claude: "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  qwen: "Qwen Code",
  opencode: "OpenCode",
  kilo: "Kilo",
  crush: "Crush",
  cursor: "Cursor",
  copilot: "Copilot",
  cline: "Cline",
  roo: "Roo Code",
  continue: "Continue",
  goose: "Goose",
  zed: "Zed",
  droid: "Droid",
  kimi: "Kimi",
  pi: "Pi",
  amp: "Amp",
  aider: "Aider",
  junie: "Junie",
  auggie: "Auggie",
  grok: "Grok",
  kiro: "Kiro",
  devin: "Devin",
  windsurf: "Windsurf",
  warp: "Warp",
  trae: "Trae",
  qoder: "Qoder",
  vibe: "Vibe",
  letta: "Letta",
  rovo: "Rovo",
  antigravity: "Antigravity",
  codebuff: "Codebuff",
  openhands: "OpenHands",
  omniget: "OmniGet",
};

/** Ferramentas cuja importação grava uma sessão retomável. */
export const NATIVE_IMPORT = ["claude", "codex"];

export function toolName(id: string): string {
  return TOOL_NAMES[id] ?? id;
}

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  const m = (e as { message?: unknown })?.message;
  return typeof m === "string" ? m : String(e);
}

export function totalTokens(u: TokenUsage | null | undefined): number {
  if (!u) return 0;
  return u.input + u.output + u.cache_read + u.cache_write + u.reasoning;
}

export function compact(n: number): string {
  const a = Math.abs(n);
  if (a >= 1e9) return `${(n / 1e9).toFixed(a >= 1e10 ? 0 : 1)}B`;
  if (a >= 1e6) return `${(n / 1e6).toFixed(a >= 1e7 ? 0 : 1)}M`;
  if (a >= 1e3) return `${(n / 1e3).toFixed(a >= 1e4 ? 0 : 1)}k`;
  return String(Math.round(n));
}

export function usd(v: number | null | undefined): string {
  if (v == null || !Number.isFinite(v)) return "—";
  if (v === 0) return "$0";
  if (Math.abs(v) < 0.01) return `$${v.toFixed(4)}`;
  if (Math.abs(v) >= 1000) return `$${Math.round(v).toLocaleString()}`;
  return `$${v.toFixed(2)}`;
}

export function pct(v: number, digits = 0): string {
  return `${(v * (v <= 1 ? 100 : 1)).toFixed(digits)}%`;
}

export function duration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0s";
  const s = Math.round(ms / 1000);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

export function toMs(ts: string | null | undefined): number {
  if (!ts) return 0;
  const n = Date.parse(ts);
  return Number.isFinite(n) ? n : 0;
}

export function relTime(ms: number, now = Date.now()): string {
  if (!ms) return "—";
  let diff = Math.round((ms - now) / 1000);
  // Relógios de arquivo um pouco à frente do "agora" da tela não viram futuro.
  if (diff > 0 && diff < 300) diff = 0;
  const abs = Math.abs(diff);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" });
  if (abs < 60) return rtf.format(diff, "second");
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(diff / 86400), "day");
  return new Date(ms).toLocaleDateString();
}

export function dateTime(ts: string | null | undefined): string {
  const ms = toMs(ts);
  return ms ? new Date(ms).toLocaleString() : "—";
}

export function projectName(path: string | null | undefined): string {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** `YYYY-MM-DD` local. */
export function isoDay(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export type RangeKey = "7d" | "30d" | "90d" | "year" | "all";
export const RANGE_KEYS: RangeKey[] = ["7d", "30d", "90d", "year", "all"];

export function rangeFrom(key: RangeKey, now = new Date()): string | null {
  const d = new Date(now);
  switch (key) {
    case "7d":
      d.setDate(d.getDate() - 6);
      return isoDay(d);
    case "30d":
      d.setDate(d.getDate() - 29);
      return isoDay(d);
    case "90d":
      d.setDate(d.getDate() - 89);
      return isoDay(d);
    case "year":
      d.setDate(d.getDate() - 364);
      return isoDay(d);
    default:
      return null;
  }
}

/** Todos os dias entre `from` e `to` (inclusivo), para eixo sem buracos. */
export function daySpan(from: string, to: string): string[] {
  const out: string[] = [];
  const [fy, fm, fd] = from.split("-").map(Number);
  const [ty, tm, td] = to.split("-").map(Number);
  const cur = new Date(fy, fm - 1, fd);
  const end = new Date(ty, tm - 1, td);
  let guard = 0;
  while (cur <= end && guard++ < 4000) {
    out.push(isoDay(cur));
    cur.setDate(cur.getDate() + 1);
  }
  return out;
}

export function sessionHref(tool: string, id: string, turn?: number | null): string {
  const base = `/llm/sessions/${encodeURIComponent(tool)}/${encodeURIComponent(id)}`;
  return turn != null ? `${base}?turn=${turn}` : base;
}

// ---- preferências por espectador (conveniência; falha em silêncio) ----

export function prefGet(key: string, fallback: string): string {
  try {
    return localStorage.getItem(`central.sessions.${key}`) ?? fallback;
  } catch {
    return fallback;
  }
}

export function prefSet(key: string, value: string): void {
  try {
    localStorage.setItem(`central.sessions.${key}`, value);
  } catch {
    /* sem storage: a preferência só vale nesta tela */
  }
}

/** Interpola `{{x}}` e `{x}` (as chaves do núcleo vieram com chave simples). */
export function fill(text: string, vars: Record<string, string | number>): string {
  return text.replace(/\{\{?\s*(\w+)\s*\}?\}/g, (m, k) => (k in vars ? String(vars[k]) : m));
}

export const TOKEN_KINDS = ["input", "output", "cache_read", "cache_write", "reasoning"] as const;
export type TokenKind = (typeof TOKEN_KINDS)[number];
