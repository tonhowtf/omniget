/**
 * Test fixture of the chat store (not imported by the app): a roster and the
 * exact `TurnEvent` script a backend turn streams, so the store tests can
 * drive a turn without a backend. It used to live in `$lib/llm/fake-turn` as
 * the demo mode of the store; the app no longer invents a roster or an
 * answer when the backend fails (3.9, U4).
 */
import type { AgentDef, TurnEvent, Usage } from "$lib/llm/types";

export interface FakeTurnOptions {
  /** The whole answer; it is cut into `chunks` text deltas. */
  answer: string;
  chunks?: number;
  /** Emit a tool call block before the text. */
  tool?: { id: string; name: string; input: string } | null;
  thinking?: string;
  usage?: Partial<Usage>;
}

/** Splits `text` into at most `chunks` pieces without breaking surrogate pairs. */
export function splitChunks(text: string, chunks: number): string[] {
  const points = Array.from(text);
  const n = Math.max(1, Math.min(chunks, points.length || 1));
  const size = Math.ceil(points.length / n);
  const out: string[] = [];
  for (let i = 0; i < points.length; i += size) {
    out.push(points.slice(i, i + size).join(""));
  }
  return out.length > 0 ? out : [""];
}

/** The full event script of one fake turn, in wire order. Pure. */
export function fakeTurnEvents(requestId: string, opts: FakeTurnOptions): TurnEvent[] {
  const events: TurnEvent[] = [{ type: "started", request_id: requestId }];
  if (opts.thinking) {
    for (const piece of splitChunks(opts.thinking, 3)) {
      events.push({ type: "thinking_delta", text: piece });
    }
  }
  if (opts.tool) {
    events.push({ type: "tool_call_start", id: opts.tool.id, name: opts.tool.name });
    for (const piece of splitChunks(opts.tool.input, 2)) {
      events.push({ type: "tool_call_delta", id: opts.tool.id, input_json_delta: piece });
    }
    events.push({ type: "tool_call_end", id: opts.tool.id });
  }
  for (const piece of splitChunks(opts.answer, opts.chunks ?? 24)) {
    events.push({ type: "text_delta", text: piece });
  }
  const usage: Usage = {
    input_tokens: 412,
    output_tokens: Math.max(1, Math.round(Array.from(opts.answer).length / 4)),
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    first_token_ms: 180,
    total_ms: 1420,
    cost_usd: 0.0021,
    ...opts.usage,
  };
  events.push({ type: "usage", usage });
  events.push({ type: "finished", reason: "stop" });
  return events;
}

const T = {
  purple: [199, 125, 255] as [number, number, number],
  blue: [90, 169, 255] as [number, number, number],
  green: [76, 217, 100] as [number, number, number],
  orange: [255, 179, 64] as [number, number, number],
  teal: [72, 207, 223] as [number, number, number],
};

/** A roster with every shape the store handles (roles, routes, a CLI runtime). */
export const TEST_ROSTER: AgentDef[] = [
  {
    id: "omni",
    name: "Omni",
    role: "coordinator",
    system_prompt: "You coordinate the other agents.",
    model: { policy: "fixed", model: { provider: "anthropic", model: "claude-opus-5" } },
    tools: [
      { source: "internal", name: "downloads.enqueue", mode: "auto" },
      { source: "internal", name: "tools.run", mode: "ask" },
    ],
    skills: [],
    budget: { usd_per_day: 5, tokens_per_turn: 32000, max_tool_calls_per_turn: 8 },
    runtime: { kind: "native" },
    skin: { id: "omni-default", tint: T.purple },
  },
  {
    id: "scout",
    name: "Scout",
    role: "worker",
    system_prompt: "You find things on the web.",
    model: {
      policy: "route",
      chain: [
        { runtime: "native", provider: "openrouter", model: "google/gemini-flash", max_cost_per_1k: 0.002, min_context: 32000 },
        { runtime: "native", provider: "ollama", model: "qwen3:8b", max_cost_per_1k: 0, min_context: 16000 },
      ],
    },
    tools: [{ source: "mcp", server: "fetch", tool: "fetch", mode: "auto" }],
    skills: ["research"],
    budget: { usd_per_day: 1, tokens_per_turn: 16000, max_tool_calls_per_turn: 12 },
    runtime: { kind: "native" },
    skin: { id: "omni-default", tint: T.blue },
  },
  {
    id: "librarian",
    name: "Librarian",
    role: "worker",
    system_prompt: "You keep the library tidy.",
    model: { policy: "fixed", model: { provider: "openai", model: "gpt-5-mini" } },
    tools: [{ source: "internal", name: "library.index", mode: "auto" }],
    skills: [],
    budget: { usd_per_day: 0.5, tokens_per_turn: 8000, max_tool_calls_per_turn: 4 },
    runtime: { kind: "native" },
    skin: { id: "omni-default", tint: T.green },
  },
  {
    id: "critic",
    name: "Critic",
    role: "advisor",
    system_prompt: "You review answers before they ship.",
    model: { policy: "fixed", model: { provider: "deepseek", model: "deepseek-reasoner" } },
    tools: [],
    skills: [],
    budget: { usd_per_day: 0.5, tokens_per_turn: 12000, max_tool_calls_per_turn: 0 },
    runtime: { kind: "native" },
    skin: { id: "omni-default", tint: T.orange },
  },
  {
    id: "shellhand",
    name: "Shellhand",
    role: { custom: "runner" },
    system_prompt: "You run the local CLI agents.",
    model: { policy: "fixed", model: { provider: "custom", model: "claude-code" } },
    tools: [],
    skills: [],
    budget: {},
    runtime: { kind: "cli", cli: "claude", account: "local" },
    skin: { id: "omni-default", tint: T.teal },
  },
];
