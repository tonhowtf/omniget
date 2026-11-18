export type TermLod = 'text' | 'mini' | 'card';

const AGENT_STATES = [
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
  'limit',
  'refused',
] as const;
export type AgentStateName = (typeof AGENT_STATES)[number];

export const AGENT_SOURCES = ['none', 'hook', 'osc', 'title', 'heuristic', 'exit'] as const;

export function agentStateFromWire(n: number): AgentStateName {
  return AGENT_STATES[n] ?? 'unknown';
}

export function agentStateFromName(s: string | null | undefined): AgentStateName {
  if (!s) return 'unknown';
  const snake = s
    .replace(/([a-z])([A-Z])/g, '$1_$2')
    .replace(/-/g, '_')
    .toLowerCase();
  return (AGENT_STATES as readonly string[]).includes(snake) ? (snake as AgentStateName) : 'unknown';
}

class TermLive {
  title = $state('');
  cwd = $state('');
  lod = $state<TermLod>('card');
  agent = $state<AgentStateName>('unknown');
  agentSource = $state('');
  agentSince = $state(0);
  agentMessage = $state('');
  progress = $state<number | null>(null);
  altScreen = $state(false);
  displayOffset = $state(0);
  historyLen = $state(0);
  hasSelection = $state(false);
  exited = $state<{ code: number | null; signal: string | null } | null>(null);
  error = $state('');
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

