/**
 * TypeScript mirror of the Rust LLM contract.
 *
 * Hand-written from `src-tauri/omniget-core/src/core/llm/types.rs` and
 * `.../llm/agent.rs` (no codegen). Serde attributes that matter here:
 *   - `ContentPart`, `TurnEvent`, `ToolSource`, `ModelPolicy`, `RuntimeKind`
 *     and `CandidateRuntime` are internally tagged; the tag is `type`,
 *     `source`, `policy`, `kind` or `runtime` as noted on each type.
 *   - every variant name is `snake_case` on the wire.
 * Keep this file in step with the Rust side; a mismatch is a silent bug.
 */

export type ProviderId = string;

export interface ModelRef {
  provider: ProviderId;
  model: string;
}

export type Role = "system" | "user" | "assistant" | "tool";

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image"; mime: string; data_b64: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error: boolean };

export interface Message {
  role: Role;
  parts: ContentPart[];
}

export interface ToolSpec {
  name: string;
  description: string;
  input_schema: unknown;
}

export interface GenParams {
  temperature?: number | null;
  top_p?: number | null;
  max_tokens?: number | null;
  reasoning_effort?: string | null;
  stop?: string[];
  extra?: Record<string, unknown>;
}

export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  first_token_ms?: number | null;
  total_ms: number;
  cost_usd?: number | null;
}

export type FinishReason =
  | "stop"
  | "length"
  | "tool_use"
  | "content_filter"
  | "cancelled"
  | "other";

/** Mirrors `LlmError` in `core/llm/error.rs`. */
export interface LlmError {
  code: string;
  message: string;
  retryable: boolean;
  retry_after_ms?: number | null;
}

/** Mirrors `TurnEvent` (tag `type`, snake_case). */
export type TurnEvent =
  | { type: "started"; request_id: string }
  | { type: "text_delta"; text: string }
  | { type: "thinking_delta"; text: string }
  | { type: "tool_call_start"; id: string; name: string }
  | { type: "tool_call_delta"; id: string; input_json_delta: string }
  | { type: "tool_call_end"; id: string }
  | { type: "usage"; usage: Usage }
  | { type: "tool_result"; id: string; content: string; is_error: boolean }
  | {
      type: "prune_receipt";
      request: number;
      omitted: number;
      est_tokens_before: number;
      est_tokens_after: number;
      input_tokens: number | null;
    }
  | { type: "finished"; reason: FinishReason }
  | { type: "error"; error: LlmError };

/** Payload of the `llm://turn` Tauri event. */
export interface TurnEventEnvelope {
  request_id: string;
  event: TurnEvent;
}

// ── Roster (core/llm/agent.rs) ──────────────────────────────────────────

/** `AgentRole` is externally tagged for `Custom`: `"worker"` or `{ custom: "…" }`. */
export type AgentRole =
  | "coordinator"
  | "worker"
  | "advisor"
  | { custom: string };

export type CandidateRuntime =
  | { runtime: "native"; provider: ProviderId }
  | { runtime: "cli"; account_id: string };

export type Candidate = CandidateRuntime & {
  model: string;
  max_cost_per_1k?: number | null;
  min_context?: number;
};

export type ModelPolicy =
  | { policy: "fixed"; model: ModelRef }
  | { policy: "route"; chain: Candidate[] };

export type ToolSource =
  | { source: "internal"; name: string }
  | { source: "mcp"; server: string; tool: string }
  | { source: "skill"; name: string };

export type GrantMode = "auto" | "ask" | "deny";

export type ToolGrant = ToolSource & { mode: GrantMode };

export interface Budget {
  usd_per_day?: number | null;
  tokens_per_turn?: number | null;
  max_tool_calls_per_turn?: number;
}

export type RuntimeKind =
  | { kind: "native" }
  | { kind: "cli"; cli: string; account: string }
  | { kind: "acp"; command: string; args?: string[] };

export interface Skin {
  id: string;
  /** RGB 0–255, same shape as the profile skin tint. */
  tint: [number, number, number];
}

export interface AgentDef {
  id: string;
  name: string;
  role: AgentRole;
  system_prompt: string;
  model: ModelPolicy;
  tools?: ToolGrant[];
  skills?: string[];
  budget?: Budget;
  runtime: RuntimeKind;
  skin?: Skin | null;
}

// ── UI-side shapes (not on the Rust wire) ───────────────────────────────

export interface ToolCallView {
  id: string;
  name: string;
  /** Concatenated `input_json_delta`; may be partial while streaming. */
  input: string;
  done: boolean;
}

/** Who wrote a message. Direct chats: the user or the conversation's bot. */
export type AuthorKind = "user" | "bot" | "system";

export interface MessageAuthor {
  kind: AuthorKind;
  /** The bot that wrote it (rooms have several). */
  botId?: string | null;
}

export interface ChatMessage {
  id: string;
  role: Role;
  text: string;
  /** Typed authorship; absent on old direct-chat messages (the agent or you). */
  author?: MessageAuthor;
  /** The message this one answers (room). */
  replyTo?: string | null;
  /** The run (request id) that produced it. */
  runId?: string | null;
  /** The delegated task it belongs to, when a member worked for another. */
  taskId?: string | null;
  /** Room status: `done`, `failed`, `cancelled`, `interrupted`. */
  status?: string;
  /** Set on assistant messages that ran tools. */
  toolCalls?: ToolCallView[];
  thinking?: string;
  usage?: Usage | null;
  /** `provider/model` at the time the message was produced. */
  modelLabel?: string;
  error?: LlmError | null;
  finish?: FinishReason | null;
}

/** `projectless` = personal (no folder, no project tools); `project` = one folder. */
export type ContextKind = "projectless" | "project";

export interface ConversationContext {
  kind: ContextKind;
  path: string | null;
}

export interface Conversation {
  id: string;
  /**
   * Direct chat: its bot. Room: the member that answers by default
   * (coordinator, else default bot, else first member).
   */
  agentId: string;
  /** Missing on chats created before rooms existed: they are direct. */
  kind?: "direct" | "group";
  /** True once the backend has the transcript (loaded lazily on open). */
  loaded?: boolean;
  title: string;
  messages: ChatMessage[];
  /** Overrides the agent's model for this conversation (`llm_switch_model`). */
  model?: ModelRef | null;
  updatedAtMs: number;
}

/** Payload of `llm://tool-ask` (re-emitted `BusEvent::ToolAsk`). */
export interface ToolAsk {
  agent: string;
  request_id: string;
  tool_call_id: string;
  tool: string;
  /** The command, the path or the head of the patch being asked about. */
  preview?: string;
}

export interface TurnState {
  requestId: string;
  conversationId: string;
  agentId: string;
  text: string;
  thinking: string;
  toolCalls: ToolCallView[];
  usage: Usage | null;
  error: LlmError | null;
  startedAtMs: number;
  /** True between `llm_turn_start` and the first `started` event. */
  starting: boolean;
}

// ── Helpers on the contract (pure) ──────────────────────────────────────

export function roleLabelKey(role: AgentRole): string {
  if (typeof role === "string") return `llm.role.${role}`;
  return "llm.role.custom";
}

export function roleText(role: AgentRole): string | null {
  return typeof role === "string" ? null : role.custom;
}

/** `provider/model` for the header and the inspector, or `""` when unknown. */
export function modelLabel(policy: ModelPolicy | null | undefined): string {
  if (!policy) return "";
  if (policy.policy === "fixed") return `${policy.model.provider}/${policy.model.model}`;
  const first = policy.chain[0];
  return first ? `${candidateProvider(first)}/${first.model}` : "";
}

export function candidateProvider(candidate: Candidate): string {
  return candidate.runtime === "native" ? candidate.provider : candidate.account_id;
}

export function modelRefLabel(ref: ModelRef | null | undefined): string {
  return ref ? `${ref.provider}/${ref.model}` : "";
}

/** Effective model of a conversation: the per-chat override wins. */
export function effectiveModelLabel(
  conversation: Conversation | null | undefined,
  agent: AgentDef | null | undefined,
): string {
  if (conversation?.model) return modelRefLabel(conversation.model);
  return modelLabel(agent?.model);
}

export function agentTint(agent: AgentDef | null | undefined): [number, number, number] {
  return agent?.skin?.tint ?? [110, 139, 255];
}


// ── Rooms (core/assist/groups) ──────────────────────────────────────────

export interface RoomMember {
  bot: string;
  role: string;
}

export interface RoomLimits {
  max_depth: number;
  max_delegations_per_round: number;
  max_turns_per_round: number;
  max_concurrency: number;
  max_task_ms: number;
  max_tokens_per_round: number | null;
  unknown_run_tokens: number;
}

export type MentionPolicy = "mentions_or_coordinator" | "mentions_only";

export interface Room {
  id: string;
  title: string;
  members: RoomMember[];
  coordinator: string | null;
  default_bot: string | null;
  mention_policy: MentionPolicy;
  limits: RoomLimits;
  round: number;
  created_ms: number;
  updated_ms: number;
}

export interface RoomDraft {
  title: string;
  members: RoomMember[];
  coordinator?: string | null;
  default_bot?: string | null;
  mention_policy?: MentionPolicy;
  limits?: RoomLimits | null;
}

/** `store::RoomMessage` on the wire (snake_case). */
export interface RoomMessageWire {
  id: string;
  room: string;
  seq: number;
  author: AuthorKind;
  bot_id: string | null;
  reply_to: string | null;
  run_id: string | null;
  task_id: string | null;
  round: number;
  text: string;
  status: string;
  created_ms: number;
}

export interface RoomRun {
  run_id: string;
  room: string;
  bot: string;
  task_id: string | null;
  reply_to: string | null;
  round: number;
  state: string;
  tokens: number | null;
  started_ms: number;
  finished_ms: number | null;
}

export interface RoomTask {
  id: string;
  room: string;
  parent_task: string | null;
  creator_bot: string;
  creator_run: string | null;
  recipient: string;
  question: string;
  context: string;
  scopes: string[];
  deliverable: string;
  limit_ms: number;
  depth: number;
  round: number;
  state: "pending" | "claimed" | "completed" | "failed" | "cancelled" | "interrupted";
  claim_run: string | null;
  result: string | null;
  error: string | null;
  message_id: string | null;
  created_ms: number;
  finished_ms: number | null;
}

export interface RoomSendOutcome {
  message: RoomMessageWire;
  round: number;
  started: { bot: string; run_id: string }[];
  skipped: { bot: string; reason: string }[];
}

/** Room ids are `grp-` + 12 hex digits; a member session is `<room>~<bot>`. */
export function isRoomId(id: string | null | undefined): boolean {
  return !!id && /^grp-[0-9a-f]{12}$/.test(id);
}

export function isRoomSession(id: string | null | undefined): boolean {
  return !!id && /^grp-[0-9a-f]{12}[~_]/.test(id);
}

/** Same rule as `tasks::handle_of` in Rust: what `@` inserts for a bot. */
export function handleOf(name: string): string {
  let out = "";
  for (const raw of name.trim()) {
    const c = raw.toLowerCase();
    if (/[a-z0-9_-]/.test(c)) out += c;
    else if (/\s/.test(c) && !out.endsWith("-")) out += "-";
  }
  return out.replace(/^-+|-+$/g, "");
}

/** One character folded like `tasks::fold_char`: lowercase, accents off. */
function foldChar(c: string): string {
  return c.normalize("NFD").charAt(0).toLowerCase();
}

/**
 * Mirror of `tasks::mentions`: members mentioned in `text`, in order, no
 * repeats. `@` followed by an id, a handle (what the autocomplete inserts) or
 * the display name shown in the room ("@Companheiro de leitura"), compared
 * without case or accents; the longest match wins; it must end at a word
 * boundary.
 */
export function mentionedMembers(text: string, members: { id: string; name: string }[]): string[] {
  const out: string[] = [];
  const chars = [...text];
  const isWord = (c: string | undefined) => !!c && /[\p{L}\p{N}_-]/u.test(c);
  const matchAt = (start: number, candidate: string): number | null => {
    const cand = [...candidate].map(foldChar);
    if (cand.length === 0 || start + cand.length > chars.length) return null;
    for (let k = 0; k < cand.length; k++) if (foldChar(chars[start + k]) !== cand[k]) return null;
    return isWord(chars[start + cand.length]) ? null : cand.length;
  };
  let i = 0;
  while (i < chars.length) {
    if (chars[i] === "@" && (i === 0 || !/[\p{L}\p{N}]/u.test(chars[i - 1]))) {
      const start = i + 1;
      let best: { len: number; id: string } | null = null;
      for (const m of members) {
        for (const cand of [m.id, handleOf(m.name), m.name.trim()]) {
          const len = matchAt(start, cand);
          if (len !== null && (!best || len > best.len)) best = { len, id: m.id };
        }
      }
      if (best) {
        if (!out.includes(best.id)) out.push(best.id);
        i = start + best.len;
      } else i = start;
    } else i += 1;
  }
  return out;
}

/** Mirror of `tasks::route`: who answers a user message in a room. */
export function routeTargets(room: Room, text: string, names: Record<string, string>): string[] {
  const members = room.members.map((m) => ({ id: m.bot, name: names[m.bot] ?? "" }));
  const mentioned = mentionedMembers(text, members);
  if (mentioned.length > 0) return mentioned;
  if (room.mention_policy === "mentions_only") return [];
  const fallback = room.coordinator ?? room.default_bot ?? room.members[0]?.bot ?? null;
  return fallback ? [fallback] : [];
}

/** The `@query` being typed right before the caret, for autocomplete. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const match = /(^|\s)@([A-Za-z0-9_-]*)$/.exec(before);
  if (!match) return null;
  return { start: caret - match[2].length - 1, query: match[2].toLowerCase() };
}

// ── Run status (`assist://run`, briefing contract 5) ────────────────────

export type RunState =
  | "queued"
  | "preparing"
  | "running"
  | "waiting_user"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted"
  | "unknown";

export interface RunUpdate {
  run_id: string;
  conversation_id: string;
  bot_id: string;
  state: RunState;
  error?: string | null;
  resume_kind?: "native" | "replay" | "new" | null;
}

/** What the composer says about the conversation's work. */
export type TurnStatus = "idle" | "working" | "waiting_you" | "interrupted" | "failed" | "cancelled" | "completed";

/** A message the user sent while a turn was running: waits, visible, cancelable. */
export interface QueuedMessage {
  id: string;
  conversationId: string;
  text: string;
  /** `waiting` until the turn ends; `failed` if it could not start then. */
  state: "waiting" | "failed";
}
