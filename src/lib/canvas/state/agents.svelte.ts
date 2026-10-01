// Canonical agent state per terminal (briefing "Terminal (core::term)"):
// fed by `term://state` (keyed by terminal id) and `canvas://node-state`
// (keyed by node id, from the cords bus). The canvas, cards, sprites and the
// "next needs you" shortcut all read from here.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type AgentState =
  | 'idle'
  | 'thinking'
  | 'working'
  | 'waitingPeer'
  | 'blocked'
  | 'success'
  | 'failed'
  | 'compacting'
  | 'looping'
  | 'unknown';

export interface AgentSignal {
  state: AgentState;
  source?: string;
  since_ms?: number;
  message?: string;
  /** Optional live details some sources carry. */
  contextTokens?: number;
  contextLimit?: number;
  costUsd?: number;
  lastLine?: string;
}

const TERM_STATE_EVENT = 'term://state';
const NODE_STATE_EVENT = 'canvas://node-state';

/** Munder Difflin palette, one channel per question (estudo 97 §4). */
export const STATE_COLORS: Record<AgentState, number> = {
  idle: 0xa199ab,
  thinking: 0x4f9faf,
  working: 0xdcab3c,
  waitingPeer: 0x7f8fc9,
  blocked: 0xd96a62,
  success: 0x5ca97a,
  failed: 0xc0504d,
  compacting: 0xb48ee0,
  looping: 0xe8a33c,
  unknown: 0x8a8a8a,
};

export function stateCss(state: AgentState | undefined): string {
  const c = STATE_COLORS[state ?? 'unknown'] ?? STATE_COLORS.unknown;
  return `#${c.toString(16).padStart(6, '0')}`;
}

/** Normalises whatever casing the Rust side sends (`Blocked`, `waiting_peer`...). */
export function normaliseState(raw: unknown): AgentState {
  const s = String(raw ?? '').replace(/[_\s-]/g, '').toLowerCase();
  switch (s) {
    case 'idle':
    case 'ready':
      return 'idle';
    case 'thinking':
      return 'thinking';
    case 'working':
    case 'busy':
      return 'working';
    case 'waitingpeer':
    case 'waiting':
      return 'waitingPeer';
    case 'blocked':
    case 'needsyou':
    case 'attention':
      return 'blocked';
    case 'success':
    case 'done':
      return 'success';
    case 'failed':
    case 'error':
      return 'failed';
    case 'compacting':
      return 'compacting';
    case 'looping':
      return 'looping';
    default:
      return 'unknown';
  }
}

interface RawStateEvent {
  id: string;
  state: unknown;
  source?: string;
  since_ms?: number;
  message?: string;
  [k: string]: unknown;
}

class AgentStates {
  /** Terminal id → signal. Replaced (not mutated) on change. */
  byTerminal = $state.raw<Record<string, AgentSignal>>({});
  /** Node id → signal (bus-derived; wins when present and fresher). */
  byNode = $state.raw<Record<string, AgentSignal>>({});
  private unlisten: UnlistenFn[] = [];
  private refs = 0;

  private toSignal(p: RawStateEvent): AgentSignal {
    // The node bus says "done" + detail "failed" for a failed turn the user
    // has not seen; "exited" carries no outcome (the terminal signal has it).
    const state = p.detail === 'failed' ? 'failed' : normaliseState(p.state);
    return {
      state,
      source: p.source,
      since_ms: typeof p.since_ms === 'number' ? p.since_ms : Date.now(),
      message: p.message,
      contextTokens: typeof p.context_tokens === 'number' ? (p.context_tokens as number) : undefined,
      contextLimit: typeof p.context_limit === 'number' ? (p.context_limit as number) : undefined,
      costUsd: typeof p.cost_usd === 'number' ? (p.cost_usd as number) : undefined,
      lastLine: typeof p.last_line === 'string' ? (p.last_line as string) : undefined,
    };
  }

  /** Starts listening; reference counted so several canvases can share it. */
  async start(): Promise<void> {
    this.refs++;
    if (this.refs > 1) return;
    try {
      this.unlisten.push(
        await listen<RawStateEvent>(TERM_STATE_EVENT, (e) => {
          if (!e.payload?.id) return;
          this.set(e.payload.id, this.toSignal(e.payload));
        }),
      );
      this.unlisten.push(
        await listen<RawStateEvent>(NODE_STATE_EVENT, (e) => {
          if (!e.payload?.id) return;
          this.byNode = { ...this.byNode, [e.payload.id]: this.toSignal(e.payload) };
        }),
      );
    } catch {
      // Outside Tauri (tests, storybook): states stay empty.
    }
  }

  stop(): void {
    this.refs = Math.max(0, this.refs - 1);
    if (this.refs > 0) return;
    for (const u of this.unlisten) u();
    this.unlisten = [];
  }

  set(terminalId: string, signal: AgentSignal): void {
    const prev = this.byTerminal[terminalId];
    if (prev && prev.state === signal.state && prev.message === signal.message && prev.lastLine === signal.lastLine) {
      return;
    }
    this.byTerminal = { ...this.byTerminal, [terminalId]: { ...prev, ...signal } };
  }

  forget(terminalId: string): void {
    if (!(terminalId in this.byTerminal)) return;
    const next = { ...this.byTerminal };
    delete next[terminalId];
    this.byTerminal = next;
  }

  /** Signal of a canvas node (by node id first, then its terminal id). */
  of(node: { id: string; data?: unknown }): AgentSignal | undefined {
    const byNode = this.byNode[node.id];
    const tid = (node.data as { terminalId?: string } | undefined)?.terminalId;
    const byTerm = tid ? this.byTerminal[tid] : undefined;
    if (byNode && byTerm) {
      // A bus state that carries no outcome ("exited", "no_signal") never
      // hides a known terminal state (an Ollama node that failed at launch).
      if (byNode.state === 'unknown' && byTerm.state !== 'unknown') return byTerm;
      return (byNode.since_ms ?? 0) >= (byTerm.since_ms ?? 0) ? byNode : byTerm;
    }
    return byNode ?? byTerm;
  }

  stateOf(node: { id: string; data?: unknown }): AgentState {
    return this.of(node)?.state ?? 'unknown';
  }
}

export const agentStates = new AgentStates();
