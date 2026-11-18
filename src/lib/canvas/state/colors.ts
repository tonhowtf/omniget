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
  | 'limit'
  | 'refused'
  | 'unknown';

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
  limit: 0xb98a2f,
  refused: 0xb5627f,
  unknown: 0x8a8a8a,
};

export function stateCss(state: AgentState | undefined): string {
  const c = STATE_COLORS[state ?? 'unknown'] ?? STATE_COLORS.unknown;
  return `#${c.toString(16).padStart(6, '0')}`;
}

export function stateVar(state: AgentState | undefined): string {
  const s = state ?? 'unknown';
  const name = s.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
  return `var(--agent-${name}, ${stateCss(s)})`;
}
