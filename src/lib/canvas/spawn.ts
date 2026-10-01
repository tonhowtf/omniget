import { invoke } from '@tauri-apps/api/core';
import type { AgentKind, CanvasNode } from './model/types';
import { terminalIdOf } from './model/types';
import { termClose, type TermInfo, } from './term/client';
import { gridSizeFor } from './term/metrics';

const DEFAULT_OLLAMA_MODEL = 'llama3.2';

const AGENT_COMMANDS: Record<AgentKind, string[]> = {
  shell: [],
  claude: ['claude'],
  codex: ['codex'],
  gemini: ['gemini'],
  opencode: ['opencode'],
  ollama: ['ollama', 'run', DEFAULT_OLLAMA_MODEL],
  native: [],
};

function splitCommand(line: string): string[] {
  const out: string[] = [];
  let cur = '';
  let has = false;
  let quote: '"' | "'" | null = null;
  const s = line.trim();
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quote === "'") {
      if (c === "'") quote = null;
      else cur += c;
    } else if (quote === '"') {
      if (c === '"') quote = null;
      else if (c === '\\' && i + 1 < s.length && '"\\$`'.includes(s[i + 1])) cur += s[++i];
      else cur += c;
    } else if (c === "'" || c === '"') {
      quote = c;
      has = true;
    } else if (c === '\\' && i + 1 < s.length) {
      cur += s[++i];
      has = true;
    } else if (/\s/.test(c)) {
      if (has || cur) out.push(cur);
      cur = '';
      has = false;
    } else {
      cur += c;
      has = true;
    }
  }
  if (quote) return s.split(/\s+/).filter(Boolean);
  if (has || cur) out.push(cur);
  return out;
}

export function agentCommand(node: CanvasNode): string[] {
  const custom = node.data.command;
  if (Array.isArray(custom) && custom.length) return (custom as unknown[]).map(String);
  if (typeof custom === 'string' && custom.trim()) return splitCommand(custom);
  const agent = agentOf(node);
  if (agent === 'ollama') {
    const model = typeof node.data.model === 'string' && node.data.model.trim() ? node.data.model.trim() : DEFAULT_OLLAMA_MODEL;
    return ['ollama', 'run', model];
  }
  return [...(AGENT_COMMANDS[agent] ?? [])];
}

function agentOf(node: CanvasNode): AgentKind {
  const a = node.data.agent;
  return typeof a === 'string' && a in AGENT_COMMANDS ? (a as AgentKind) : 'shell';
}

export function gridFor(w: number, h: number): { cols: number; rows: number } {
  return gridSizeFor(w, h);
}

const inFlight = new Map<string, Promise<TermInfo>>();

interface OpenOptions {
  floorId?: string;
  restore?: boolean;
}

export function openTerminal(node: CanvasNode, workspaceId: string, opts: OpenOptions = {}): Promise<TermInfo> {
  const tid = terminalIdOf(node);
  const running = inFlight.get(tid);
  if (running) return running;
  const p = launch(node, workspaceId, opts).finally(() => inFlight.delete(tid));
  inFlight.set(tid, p);
  return p;
}

async function launch(node: CanvasNode, workspaceId: string, opts: OpenOptions): Promise<TermInfo> {
  const { cols, rows } = gridFor(node.w, node.h);
  return invoke<TermInfo>('canvas_term_ensure', {
    workspaceId,
    floorId: opts.floorId ?? 'ground',
    nodeId: node.id,
    node,
    cols,
    rows,
    restore: opts.restore ?? false,
  });
}

export async function closeTerminal(terminalId: string, deleteHistory = true): Promise<void> {
  try {
    await termClose(terminalId, deleteHistory);
  } catch {
  }
}
