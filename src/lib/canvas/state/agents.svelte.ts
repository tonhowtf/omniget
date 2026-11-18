import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { STATE_COLORS, stateCss, stateVar, type AgentState } from './colors';

export { STATE_COLORS, stateCss, stateVar, type AgentState };

interface AgentSignal {
  state: AgentState;
  source?: string;
  since_ms?: number;
  message?: string;
  contextTokens?: number;
  contextLimit?: number;
  costUsd?: number;
  lastLine?: string;
}

const TERM_STATE_EVENT = 'term://state';
const NODE_STATE_EVENT = 'canvas://node-state';
const NODE_COST_EVENT = 'canvas://node-cost';
const OUTCOMES = new Set(['failed', 'limit', 'refused']);

export interface NodeCost {
  usd: number;
  tokens: number;
  unpriced: boolean;
}

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
    case 'limit':
      return 'limit';
    case 'refused':
      return 'refused';
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
  byTerminal = $state.raw<Record<string, AgentSignal>>({});
  byNode = $state.raw<Record<string, AgentSignal>>({});
  costs = $state.raw<Record<string, NodeCost>>({});
  private unlisten: UnlistenFn[] = [];
  private refs = 0;

  private toSignal(p: RawStateEvent): AgentSignal {
    const plain = normaliseState(p.state);
    const state = plain === 'success' && OUTCOMES.has(String(p.detail)) ? (p.detail as AgentState) : plain;
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
      this.unlisten.push(
        await listen<{ id?: string; costUsd?: number; tokens?: number; unpriced?: boolean }>(NODE_COST_EVENT, (e) => {
          const p = e.payload;
          if (!p?.id) return;
          this.costs = { ...this.costs, [p.id]: { usd: Number(p.costUsd) || 0, tokens: Number(p.tokens) || 0, unpriced: p.unpriced === true } };
        }),
      );
    } catch {
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

  of(node: { id: string; data?: unknown }): AgentSignal | undefined {
    const byNode = this.byNode[node.id];
    const tid = (node.data as { terminalId?: string } | undefined)?.terminalId;
    const byTerm = tid ? this.byTerminal[tid] : undefined;
    if (byNode && byTerm) {
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
