// Omni as the canvas companion: typed wrappers over the
// `canvas_companion_*` commands and the events `crate::canvas::companion`
// emits. Summaries arrive on `canvas://companion` (the canvas panel and the
// floating pet both listen); a dismissed one on `canvas://companion-dismiss`.

import { invoke } from '@tauri-apps/api/core';

export const EVENT_COMPANION = 'canvas://companion';
export const EVENT_COMPANION_DISMISS = 'canvas://companion-dismiss';

export type CompanionKind = 'done' | 'blocked' | 'failed' | 'manual';
export type ModelMode = 'auto' | 'local' | 'cheap' | 'heuristic';

export interface CompanionEntry {
  id: string;
  nodeId: string;
  workspaceId: string;
  floorId: string;
  terminalId: string;
  name: string;
  agent: string;
  kind: CompanionKind;
  summary: string;
  suggestions: string[];
  command?: string | null;
  /** `ollama:<model>`, `llm:<model>` or `heuristic`. */
  source: string;
  atMs: number;
  workedMs: number;
  celebrate: boolean;
  dismissed?: boolean;
  /** Only on the live event: whether the pet should show it. */
  petBalloon?: boolean;
  /** Heuristic entries: i18n keys behind the text (shown in the window's language). */
  i18n?: EntryI18n | null;
}

export interface EntryI18n {
  summary?: { key: string; vars?: Record<string, unknown> };
  suggestions?: Array<{ key?: string; text: string }>;
}

type Tr = (key: string, vars?: Record<string, unknown>) => string;

/** English defaults of the heuristic keys (the Rust side's own words). */
const H_DEFAULTS: Record<string, string> = {
  'canvas.companion.h.done': '{{name}} finished after {{duration}}.',
  'canvas.companion.h.done_last': '{{name}} finished after {{duration}}. Last line: “{{last}}”',
  'canvas.companion.h.blocked': '{{name}} is waiting for you.',
  'canvas.companion.h.blocked_last': '{{name}} is waiting for you. Last line: “{{last}}”',
  'canvas.companion.h.failed': '{{name}} hit an error.',
  'canvas.companion.h.failed_last': '{{name}} hit an error. Last line: “{{last}}”',
  'canvas.companion.h.manual': 'Here is where {{name}} is.',
  'canvas.companion.h.manual_last': 'Here is where {{name}} is. Last line: “{{last}}”',
};

/** The summary in the window's language (heuristic entries), else as sent. */
export function entrySummary(e: CompanionEntry, tr: Tr): string {
  const s = e.i18n?.summary;
  if (!s?.key) return e.summary;
  // Values are escaped: a `{{` in a terminal line must not re-enter the parser.
  const vars = Object.fromEntries(
    Object.entries(s.vars ?? {}).map(([k, v]) => [k, String(v ?? '').replace(/[{}]/g, (c) => `\\${c}`)]),
  );
  return tr(s.key, { default: H_DEFAULTS[s.key] ?? e.summary, ...vars });
}

/** The three next steps in the window's language (commands stay as they are). */
export function entrySuggestions(e: CompanionEntry, tr: Tr): string[] {
  const list = e.i18n?.suggestions;
  if (!list?.length) return e.suggestions;
  return list.map((x, i) => (x.key ? tr(x.key, { default: e.suggestions[i] ?? x.text }) : x.text));
}

export interface CompanionSettings {
  enabled: boolean;
  minWorkSecs: number;
  modelMode: ModelMode;
  ollamaUrl: string;
  ollamaModel?: string | null;
  cheapAgentId?: string | null;
  petBalloon: boolean;
  onDone: boolean;
  onBlocked: boolean;
  onFailed: boolean;
  language?: string | null;
  /** With no local model, ask an installed agent CLI headless (cheapest model). */
  cliFallback?: boolean;
  /** `claude`, `codex` or `gemini`; null = the CLI already used on the canvas. */
  cliAgent?: string | null;
}

export function getSettings(): Promise<CompanionSettings> {
  return invoke('canvas_companion_settings_get');
}

export function setSettings(settings: CompanionSettings): Promise<CompanionSettings> {
  return invoke('canvas_companion_settings_set', { settings });
}

export function history(workspaceId?: string, limit = 40): Promise<CompanionEntry[]> {
  return invoke('canvas_companion_history', { workspaceId: workspaceId ?? null, limit });
}

export function dismiss(id: string): Promise<boolean> {
  return invoke('canvas_companion_dismiss', { id });
}

/** Sends suggestion `index` (0-2, 3 = the direct command) or `text`. */
export function act(id: string, index?: number, text?: string): Promise<unknown> {
  return invoke('canvas_companion_act', { id, index: index ?? null, text: text ?? null });
}

export function summarize(nodeId: string): Promise<CompanionEntry> {
  return invoke('canvas_companion_summarize', { nodeId });
}

/** Omni's answer to a free question about the workspace. */
export interface AskAnswer {
  intent: 'free' | 'note' | 'summarize_notes' | 'summary';
  answer: string;
  sources: string[];
  source: string;
  noteId?: string;
  created?: boolean;
}

/** A free question ("what is B doing?", "note that ...", "summarize my notes"). */
export function askOmni(workspaceId: string, question: string, nodeId?: string): Promise<AskAnswer> {
  return invoke('canvas_companion_ask', { workspaceId, question, nodeId: nodeId ?? null });
}

export function localModels(): Promise<string[]> {
  return invoke('canvas_companion_models');
}

/** Loose check of an event payload (the pet trusts nothing it is sent). */
export function asEntry(raw: unknown): CompanionEntry | null {
  const e = raw as Partial<CompanionEntry> | null;
  if (!e || typeof e.id !== 'string' || typeof e.summary !== 'string') return null;
  if (typeof e.nodeId !== 'string' || typeof e.name !== 'string') return null;
  const suggestions = Array.isArray(e.suggestions)
    ? e.suggestions.filter((s): s is string => typeof s === 'string').slice(0, 3)
    : [];
  return {
    id: e.id,
    nodeId: e.nodeId,
    workspaceId: String(e.workspaceId ?? ''),
    floorId: String(e.floorId ?? ''),
    terminalId: String(e.terminalId ?? ''),
    name: e.name,
    agent: String(e.agent ?? ''),
    kind: (['done', 'blocked', 'failed', 'manual'] as const).includes(e.kind as CompanionKind)
      ? (e.kind as CompanionKind)
      : 'manual',
    summary: e.summary,
    suggestions,
    command: typeof e.command === 'string' ? e.command : null,
    source: String(e.source ?? ''),
    atMs: Number(e.atMs ?? Date.now()),
    workedMs: Number(e.workedMs ?? 0),
    celebrate: e.celebrate === true,
    dismissed: e.dismissed === true,
    petBalloon: e.petBalloon === true,
    i18n: e.i18n && typeof e.i18n === 'object' ? (e.i18n as EntryI18n) : null,
  };
}

export function errorText(e: unknown): string {
  const s = e instanceof Error ? e.message : String(e);
  const i = s.indexOf(':');
  return i > 0 && /^[A-Z_]+$/.test(s.slice(0, i)) ? s.slice(i + 1).trim() : s;
}
