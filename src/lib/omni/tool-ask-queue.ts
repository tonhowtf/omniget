export const ASK_KINDS = ['read', 'command', 'file_change', 'mcp', 'web', 'other'] as const;
export type AskKind = (typeof ASK_KINDS)[number];

export interface PendingAsk {
  agent: string;
  requestId: string;
  toolCallId: string;
  tool: string;
  kind: AskKind;
  askedMs: number | null;
  deadlineMs: number | null;
  canAlways: boolean;
  impact: AskImpact | null;
}

export interface AskImpact {
  files: number;
  added: number;
  removed: number;
  outside: boolean;
}

function cleanImpact(raw: unknown): AskImpact | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const count = (v: unknown) => (typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : null);
  const files = count(o.files);
  const added = count(o.added);
  const removed = count(o.removed);
  if (files === null || added === null || removed === null || files === 0) return null;
  return { files, added, removed, outside: o.outside === true };
}

function cleanKind(raw: unknown): AskKind {
  return ASK_KINDS.find((k) => k === raw) ?? 'other';
}

function cleanTime(raw: unknown): number | null {
  return typeof raw === 'number' && Number.isFinite(raw) && raw > 0 ? raw : null;
}

const TOOL_NAME_MAX = 64;

const ASK_QUEUE_MAX = 16;

function cleanId(raw: unknown, max: number): string | null {
  if (typeof raw !== 'string') return null;
  const kept = [...raw.trim()]
    .filter((ch) => {
      const code = ch.codePointAt(0) ?? 0;
      if (code >= 0x200b && code <= 0x200f) return false;
      if (code >= 0x202a && code <= 0x202e) return false;
      if (code >= 0x2066 && code <= 0x2069) return false;
      return code > 0x1f && code !== 0x7f && code !== 0xfeff;
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
  const rawAgent = cleanId(o.agent, 64) ?? '';
  const labelled = rawAgent.startsWith('canvas:') ? /^(.+) \((.+)\)$/.exec(tool) : null;
  return {
    agent: labelled ? labelled[2] : rawAgent,
    requestId,
    toolCallId,
    tool: labelled ? labelled[1] : tool,
    kind: cleanKind(o.kind),
    askedMs: cleanTime(o.asked_ms ?? o.askedMs),
    deadlineMs: cleanTime(o.deadline_ms ?? o.deadlineMs),
    canAlways: (o.can_always ?? o.canAlways) !== false,
    impact: cleanImpact(o.impact),
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
