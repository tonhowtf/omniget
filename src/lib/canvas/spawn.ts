// Opens and closes the PTY behind a terminal/agent node. Every open of a
// canvas terminal goes through `openTerminal` (engine spawn, TerminalNode
// mount, restart, role swap), which calls the Rust launcher
// (`canvas_term_ensure`, src-tauri/src/canvas/launch.rs), so the launch is
// the same whoever asks, the UI or the backend:
//
//   1. the agent program (`AGENT_COMMANDS`, or the node's own `data.command`);
//   2. the role, when the node has one (role args, cwd, env; Ollama gets the
//      role as its first prompt);
//   3. the cords environment (node token, bridge URL, `omniget` helper on
//      PATH) and the agent's extra args (Claude `--plugin-dir`, Codex
//      `notify`), inserted right after the program;
//   4. the floor/environment context (cwd into the floor's checkout,
//      OMNIGET_FLOOR_* and PORT, ssh/docker/wsl wrap), last on purpose: the
//      wrapper must wrap the complete argv;
//   5. `term_open`.
//
// A terminal that already exists in this app run (alive or exited) is not
// relaunched: minting a new token would revoke the running process's one.
// `agentCommand`/`termRequest` mirror steps 1 for the UI (labels, previews).

import { invoke } from '@tauri-apps/api/core';
import type { AgentKind, CanvasNode } from './model/types';
import { terminalIdOf } from './model/types';
import { termClose, type TermInfo, } from './term/client';
import { gridSizeFor } from './term/metrics';

/** Ollama model used when a node does not name one. */
const DEFAULT_OLLAMA_MODEL = 'llama3.2';

/** Command line of each agent preset; the shell preset leaves it empty (login shell). */
const AGENT_COMMANDS: Record<AgentKind, string[]> = {
  shell: [],
  claude: ['claude'],
  codex: ['codex'],
  gemini: ['gemini'],
  opencode: ['opencode'],
  ollama: ['ollama', 'run', DEFAULT_OLLAMA_MODEL],
  native: [],
};

/**
 * A command line as argv with POSIX quoting (`bash -c "echo X; exec bash"`
 * is three words), like `maestro::split_command` in Rust. Unbalanced
 * quotes fall back to whitespace.
 */
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

/** The agent's program for a node: its own command, else the preset (Ollama with the node's model). */
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

/** Terminal grid that fits a node of w×h world units. */
export function gridFor(w: number, h: number): { cols: number; rows: number } {
  return gridSizeFor(w, h);
}

const inFlight = new Map<string, Promise<TermInfo>>();

interface OpenOptions {
  floorId?: string;
  restore?: boolean;
}

/** Opens (or re-attaches to) the terminal of `node`. Concurrent calls for one terminal share the work. */
export function openTerminal(node: CanvasNode, workspaceId: string, opts: OpenOptions = {}): Promise<TermInfo> {
  const tid = terminalIdOf(node);
  const running = inFlight.get(tid);
  if (running) return running;
  const p = launch(node, workspaceId, opts).finally(() => inFlight.delete(tid));
  inFlight.set(tid, p);
  return p;
}

// The recipe (role, cords env, floor context, term_open, first prompt) runs
// in Rust (`canvas_term_ensure`, src-tauri/src/canvas/launch.rs), the same
// path routines, cords and chat use to start a node that is off screen. It
// attaches when the terminal already exists in this app run, under a lock
// per terminal, so the UI and the backend never start two processes.
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
    // Already gone.
  }
}
