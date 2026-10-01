export interface PendingAsk {
  agent: string;
  requestId: string;
  toolCallId: string;
  tool: string;
}

const TOOL_NAME_MAX = 64;

const ASK_QUEUE_MAX = 16;

function cleanId(raw: unknown, max: number): string | null {
  if (typeof raw !== 'string') return null;
  const kept = [...raw.trim()]
    .filter((ch) => {
      const code = ch.codePointAt(0) ?? 0;
      return code > 0x1f && code !== 0x7f;
    })
    .slice(0, max)
    .join('')
    .trim();
  return kept.length > 0 ? kept : null;
}

function sanitizeAsk(raw: unknown): PendingAsk | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  const requestId = cleanId(o.request_id ?? o.requestId, 128);
  const toolCallId = cleanId(o.tool_call_id ?? o.toolCallId, 128);
  const tool = cleanId(o.tool, TOOL_NAME_MAX);
  if (!requestId || !toolCallId || !tool) return null;
  return {
    agent: cleanId(o.agent, 64) ?? '',
    requestId,
    toolCallId,
    tool,
  };
}

function pushAsk(queue: readonly PendingAsk[], ask: PendingAsk): PendingAsk[] {
  if (queue.some((q) => q.toolCallId === ask.toolCallId)) return [...queue];
  if (queue.length >= ASK_QUEUE_MAX) return [...queue];
  return [...queue, ask];
}

export function resolveAsk(queue: readonly PendingAsk[], toolCallId: string): PendingAsk[] {
  return queue.filter((q) => q.toolCallId !== toolCallId);
}

export function cancelRequest(queue: readonly PendingAsk[], requestId: string): PendingAsk[] {
  return queue.filter((q) => q.requestId !== requestId);
}

export function headAsk(queue: readonly PendingAsk[]): PendingAsk | null {
  return queue.length > 0 ? queue[0] : null;
}

export function acceptAsk(queue: readonly PendingAsk[], raw: unknown): PendingAsk[] {
  const ask = sanitizeAsk(raw);
  if (!ask) return queue as PendingAsk[];
  const next = pushAsk(queue, ask);
  return next.length === queue.length ? (queue as PendingAsk[]) : next;
}

const TERMINAL_TURN_EVENTS = ['finished', 'error'] as const;

export function terminalRequestId(raw: unknown): string | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const requestId = cleanId(o.request_id ?? o.requestId, 128);
  if (!requestId) return null;
  const event = o.event;
  if (!event) return null;
  let tag: string | null = null;
  if (typeof event === 'string') tag = event;
  else if (typeof event === 'object' && !Array.isArray(event)) {
    const e = event as Record<string, unknown>;
    tag = typeof e.type === 'string' ? e.type : (Object.keys(e)[0] ?? null);
  }
  if (!tag) return null;
  const needle = tag.toLowerCase();
  return (TERMINAL_TURN_EVENTS as readonly string[]).includes(needle) ? requestId : null;
}
