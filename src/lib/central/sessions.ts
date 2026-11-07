import { invoke } from "@tauri-apps/api/core";

type TokenUsage = {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  reasoning: number;
};

type SessionMeta = {
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
  model?: string | null;
};

type ListPage = { total: number; sessions: SessionMeta[] };

export type SourceInfo = {
  tool: string;
  roots: { path: string; exists: boolean }[];
  found: boolean;
  beta: boolean;
  sessions: number;
  last_activity: string | null;
};

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

export const sessionsSources = () => invoke<SourceInfo[]>("sessions_sources");
