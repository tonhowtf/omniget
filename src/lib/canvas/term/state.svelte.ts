// Reactive per-terminal facts the node chrome shows (title, agent state,
// progress, LOD). Cells never go through here: only values that change a few
// times per second at most, and only when they actually change.

export type TermLod = 'text' | 'mini' | 'card';

/** Canonical agent states, in wire order (AGENT.state). */
export const AGENT_STATES = [
  'idle',
  'thinking',
  'working',
  'waiting_peer',
  'blocked',
  'success',
  'failed',
  'compacting',
  'looping',
  'unknown',
] as const;
export type AgentStateName = (typeof AGENT_STATES)[number];

export const AGENT_SOURCES = ['none', 'hook', 'osc', 'title', 'heuristic', 'exit'] as const;

export function agentStateFromWire(n: number): AgentStateName {
  return AGENT_STATES[n] ?? 'unknown';
}

/** Accepts the Rust enum spelled any common way (`WaitingPeer`, `waiting_peer`, `waitingPeer`). */
export function agentStateFromName(s: string | null | undefined): AgentStateName {
  if (!s) return 'unknown';
  const snake = s
    .replace(/([a-z])([A-Z])/g, '$1_$2')
    .replace(/-/g, '_')
    .toLowerCase();
  return (AGENT_STATES as readonly string[]).includes(snake) ? (snake as AgentStateName) : 'unknown';
}

export class TermLive {
  title = $state('');
  cwd = $state('');
  lod = $state<TermLod>('card');
  agent = $state<AgentStateName>('unknown');
  agentSource = $state('');
  agentSince = $state(0);
  agentMessage = $state('');
  /** 0..100, -1 indeterminate, null none. */
  progress = $state<number | null>(null);
  altScreen = $state(false);
  displayOffset = $state(0);
  historyLen = $state(0);
  hasSelection = $state(false);
  exited = $state<{ code: number | null; signal: string | null } | null>(null);
  /** Last error from the terminal commands (open, resize, ...). */
  error = $state('');
  /** Modes bitset (plain field: read by input on each key, never rendered). */
  modes = 0;
}

const lives = new Map<string, TermLive>();

export function termLive(id: string): TermLive {
  let live = lives.get(id);
  if (!live) {
    live = new TermLive();
    lives.set(id, live);
  }
  return live;
}

export function peekTermLive(id: string): TermLive | undefined {
  return lives.get(id);
}

export function dropTermLive(id: string): void {
  lives.delete(id);
}
