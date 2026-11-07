import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type GenerationState =
  | 'draft'
  | 'quoted'
  | 'awaiting_approval'
  | 'running'
  | 'done'
  | 'uncertain'
  | 'failed';

export interface ModelSummary {
  model: string;
  name: string;
  kind: string;
}

export interface ParamSpec {
  name: string;
  type: string;
  default?: unknown;
  required: boolean;
  options?: string[];
  editable: boolean;
}

export interface ModelSchema {
  model: string;
  name: string;
  kind: string;
  params: ParamSpec[];
  references: boolean;
}

export interface QuoteView {
  quoteId: string;
  credits: number;
  left: number;
  budgetLeft: number;
  balance: number;
  expiresAt: number;
  fitsBudget: boolean;
  needsApproval: boolean;
  reason?: string | null;
  references: number;
}

export interface Version {
  version: number;
  path: string;
  nodeId: string;
  credits: number;
  creditsKnown?: boolean;
  prompt: string;
  params?: Record<string, unknown>;
  model: string;
  jobId?: string;
  createdAt: number;
}

export interface BudgetView {
  workspaceId: string;
  ceiling: number;
  reserved: number;
  spent: number;
  left: number;
  autoLimit: number;
  autoPerHour: number;
  balance: number | null;
}

export interface Progress {
  workspaceId: string;
  nodeId: string;
  state?: GenerationState;
  status?: string;
  elapsedMs?: number;
  startedAt?: number;
  resultNodeId?: string;
  version?: number;
  credits?: number;
  error?: string;
}

const EVENT_PROGRESS = 'canvas://generate-progress';

let modelsCache: Promise<ModelSummary[]> | null = null;
const schemaCache = new Map<string, Promise<ModelSchema>>();

export function generateModels(): Promise<ModelSummary[]> {
  if (!modelsCache) {
    modelsCache = invoke<ModelSummary[]>('canvas_generate_models').catch((e) => {
      modelsCache = null;
      throw e;
    });
  }
  return modelsCache;
}

export function generateSchema(model: string): Promise<ModelSchema> {
  let p = schemaCache.get(model);
  if (!p) {
    p = invoke<ModelSchema>('canvas_generate_schema', { model }).catch((e) => {
      schemaCache.delete(model);
      throw e;
    });
    schemaCache.set(model, p);
  }
  return p;
}

export function generateQuote(nodeId: string): Promise<QuoteView> {
  return invoke<QuoteView>('canvas_generate_quote', { nodeId });
}

export function generateSubmit(nodeId: string, quoteId: string): Promise<unknown> {
  return invoke('canvas_generate_submit', { nodeId, quoteId });
}

export function generateReconcile(nodeId: string): Promise<unknown> {
  return invoke('canvas_generate_reconcile', { nodeId });
}

export function generateRelease(nodeId: string): Promise<unknown> {
  return invoke('canvas_generate_release', { nodeId });
}

export function generateBudget(
  workspaceId: string,
  patch: { ceiling?: number; autoLimit?: number; autoPerHour?: number } = {},
): Promise<BudgetView> {
  return invoke<BudgetView>('canvas_generate_budget', { workspaceId, ...patch });
}

export function onGenerateProgress(fn: (p: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>(EVENT_PROGRESS, (e) => fn(e.payload));
}

export function errorText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e ?? '');
  const i = raw.indexOf(': ');
  return i > 0 && /^[A-Z_]+$/.test(raw.slice(0, i)) ? raw.slice(i + 2) : raw;
}

export function formatCredits(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—';
  return Number.isInteger(n) ? String(n) : n.toFixed(n < 10 ? 2 : 1).replace(/\.?0+$/, '');
}
