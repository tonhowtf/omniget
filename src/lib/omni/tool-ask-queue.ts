/**
 * The pet's queue of pending tool permissions (`GrantMode::Ask`).
 *
 * `llm://tool-ask` (the re-emitted `BusEvent::ToolAsk`) reaches every window,
 * the pet included. The pet may therefore hold several questions at once — a
 * turn can ask twice before the user looks at the screen — so they are kept in
 * a FIFO queue and the head is the one the bubble offers.
 *
 * Everything here is pure: no Tauri, no DOM, no clock, no timers. The window
 * owns the clock and the `invoke`; this file only decides what the queue looks
 * like after an event, which is the part a test can reach.
 *
 * What the pet is allowed to show is deliberately narrow: the *name* of the
 * tool and nothing else. The arguments of a call routinely carry paths, URLs,
 * prompts and secrets, and the pet is a window that sits on top of everything
 * the user may be screen-sharing. `sanitizeAsk` drops every field that is not
 * part of the four-string contract, so a payload that grew an `input` cannot
 * leak through the bubble by accident.
 */

/** Payload of `llm://tool-ask`, already validated. */
export interface PendingAsk {
  agent: string;
  requestId: string;
  toolCallId: string;
  /** Tool name only — never arguments. */
  tool: string;
}

/**
 * Longest tool name the bubble renders. Names are dotted identifiers
 * (`fs.read`, `mcp:files:read`); anything longer is a malformed payload, not a
 * name, and is clipped rather than allowed to paint over the desktop.
 */
const TOOL_NAME_MAX = 64;

/**
 * How many questions the queue holds. A turn that asks more than this is
 * either looping or hostile; the oldest are what the user is actually looking
 * at, so the *newest* beyond the bound are dropped and the head keeps moving.
 */
const ASK_QUEUE_MAX = 16;

function cleanId(raw: unknown, max: number): string | null {
  if (typeof raw !== 'string') return null;
  const kept = [...raw.trim()]
    .filter((ch) => {
      const code = ch.codePointAt(0) ?? 0;
      // Control characters only; a name with an accent is still a name.
      return code > 0x1f && code !== 0x7f;
    })
    .slice(0, max)
    .join('')
    .trim();
  return kept.length > 0 ? kept : null;
}

/**
 * Validate an `llm://tool-ask` payload. Returns `null` when it cannot be
 * answered — a question with no `tool_call_id` is one the pet could never
 * resolve, so showing it would be a button that does nothing.
 *
 * Accepts the snake_case the Rust side emits.
 */
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

/**
 * Append an ask. FIFO, and idempotent on `tool_call_id`: the same question
 * re-emitted (a replayed event, a second listener) must not queue twice.
 */
function pushAsk(queue: readonly PendingAsk[], ask: PendingAsk): PendingAsk[] {
  if (queue.some((q) => q.toolCallId === ask.toolCallId)) return [...queue];
  if (queue.length >= ASK_QUEUE_MAX) return [...queue];
  return [...queue, ask];
}

/**
 * Drop one question, by `tool_call_id`. Answering a question that is no longer
 * queued (because `/llm` got there first) is a no-op, not an error: that is the
 * idempotency the two surfaces need to share one broker.
 */
export function resolveAsk(queue: readonly PendingAsk[], toolCallId: string): PendingAsk[] {
  return queue.filter((q) => q.toolCallId !== toolCallId);
}

/**
 * Drop every question of a turn. The pet calls this when the turn finishes,
 * errors or is cancelled: the broker stopped waiting, so a bubble offering to
 * allow the call is offering something that cannot happen any more.
 */
export function cancelRequest(queue: readonly PendingAsk[], requestId: string): PendingAsk[] {
  return queue.filter((q) => q.requestId !== requestId);
}

/** The question the bubble shows: the oldest one still pending. */
export function headAsk(queue: readonly PendingAsk[]): PendingAsk | null {
  return queue.length > 0 ? queue[0] : null;
}

/**
 * Feed a raw `llm://tool-ask` payload straight into the queue. Returns the same
 * array (by identity) when the payload is unusable, so a caller can skip a
 * re-render on garbage.
 */
export function acceptAsk(queue: readonly PendingAsk[], raw: unknown): PendingAsk[] {
  const ask = sanitizeAsk(raw);
  if (!ask) return queue as PendingAsk[];
  const next = pushAsk(queue, ask);
  return next.length === queue.length ? (queue as PendingAsk[]) : next;
}

/**
 * Turn events that end a turn, and therefore every ask of that `request_id`.
 * `llm://turn` carries `{ request_id, event: { type, ... } }`; the pet only
 * needs to know which types are terminal.
 */
const TERMINAL_TURN_EVENTS = ['finished', 'error'] as const;

/**
 * Read `{ request_id, event }` off `llm://turn` and say whether it ends a turn.
 * Returns the `request_id` to cancel, or `null`.
 *
 * Tolerant of both the tagged shape (`{ type: "finished" }`) and the plain
 * externally-tagged one (`{ "Finished": {...} }`) serde can produce, because
 * the pet does not own that serialisation and must not break when it changes.
 */
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
