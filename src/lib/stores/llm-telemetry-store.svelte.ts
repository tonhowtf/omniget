/**
 * Observatory telemetry store.
 *
 * Reads the two things the Observatory page draws:
 *   - `llm_telemetry_snapshot()` — one full snapshot, on demand (page open,
 *     refresh button);
 *   - the `llm://telemetry` event — pushed by the Rust side at 2 Hz **while a
 *     turn is alive** and at 0 Hz in idle (contract of `f2-llm-commands` §5).
 *
 * Consequence for the budget: this file never creates a timer. No
 * `setInterval`, no `setTimeout`, no `requestAnimationFrame`. The cadence is
 * the backend's, so a closed section costs exactly zero.
 *
 * A command that fails leaves the store "unavailable" and the page renders an
 * empty state with the error code.
 * `FAKE` (see `fakeSnapshot`) feeds deterministic data to the tests and to the
 * screenshot harness (`?demo=1`), and is never used to fill a real screen.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Contract (mirrors what `llm_telemetry_snapshot` must return)
// ---------------------------------------------------------------------------

/** One sample of an agent's activity. `t_ms` is a Unix epoch in milliseconds. */
interface TelemetryPoint {
  t_ms: number;
  /** Output tokens produced inside this sample. */
  tokens_out: number;
  /** Input tokens consumed inside this sample. */
  tokens_in: number;
  /** Output tokens per second measured in this sample. */
  tps: number;
  /** USD spent inside this sample. */
  cost_usd: number;
}

export type AgentState = "idle" | "streaming" | "waiting_tool" | "error";

/** A line in the agent timeline. `Rerouted` is the "trocou para X" of §9.2. */
interface AgentEvent {
  t_ms: number;
  kind: "turn_started" | "turn_finished" | "rerouted" | "error" | "budget";
  /** Stable code when the event carries one (`ERR_LLM_RATE`, …). */
  code?: string | null;
  /** Already-resolved text (model name, target account). Never a sentence. */
  detail?: string | null;
}

export interface AgentTelemetry {
  agent_id: string;
  agent_name: string;
  provider: string;
  model: string;
  state: AgentState;
  turns: number;
  errors: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  /** Median time to the first token over the window, in ms. */
  first_token_ms_p50: number | null;
  /** Last measured time to the first token, in ms. */
  first_token_ms_last: number | null;
  /** Output tokens per second on the last turn. */
  tps_last: number | null;
  cost_usd: number;
  /** Context Guardian: tokens currently held by the conversation. */
  context_used_tokens: number;
  /** Context Guardian: the model's window. 0 = unknown, guardian hides. */
  context_window: number;
  /**
   * True while the number is the `ai_tokens.rs` heuristic (before the send);
   * false once the provider's real `usage` came back.
   */
  context_estimated: boolean;
  last_error_code: string | null;
  /** Live turn, when there is one: enables "cancel" in the diagnostics panel. */
  request_id?: string | null;
  history: TelemetryPoint[];
  events: AgentEvent[];
}

/** Accounts and BYOK budgets, as the `QuotaMeter` needs them (plan §9.2). */
export interface QuotaStatus {
  id: string;
  label: string;
  kind: "cli" | "byok";
  /** 0..1 of the window/budget already spent. */
  used: number;
  /** Epoch ms of the next window flip, when the source reports one. */
  resets_at_ms: number | null;
  /** Where the number came from; `estimated` is labelled in the UI. */
  source: "reported" | "estimated";
  /** Epoch ms when the quota runs out at the current pace, if projectable. */
  exhausts_at_ms: number | null;
  /** True for the candidate the router is using right now. */
  active: boolean;
}

export interface CostBucket {
  /** `YYYY-MM-DD`, already in the local day of the backend. */
  day: string;
  cost_usd: number;
  calls: number;
}

interface TelemetrySnapshot {
  ts_ms: number;
  agents: AgentTelemetry[];
  quotas: QuotaStatus[];
  cost_by_day: CostBucket[];
  /** The fallback chain in router order; ids point into `quotas`. */
  fallback_chain: string[];
}

// ---------------------------------------------------------------------------
// Pure helpers (tested in llm-telemetry-store.test.ts)
// ---------------------------------------------------------------------------

/** Windows offered by the page head, in ms. */
export const WINDOWS = {
  "1h": 3_600_000,
  "6h": 21_600_000,
  "24h": 86_400_000,
} as const;

export type WindowKey = keyof typeof WINDOWS;

/** Hard cap on SVG points, from the budget (§5 of the prompt). */
export const MAX_POINTS = 200;

interface Aggregated {
  /** Bucket start, epoch ms. */
  t_ms: number;
  tokens_out: number;
  tokens_in: number;
  /** Mean tps of the samples in the bucket; 0 when the bucket is empty. */
  tps: number;
  cost_usd: number;
  samples: number;
}

/**
 * Fold raw samples into at most `buckets` equal slices of `[now - span, now]`.
 * Counters (tokens, cost) sum; rates (tps) average over the samples that
 * actually landed in the bucket, so an idle gap reads as 0 instead of
 * dragging the previous value forward.
 */
export function aggregateWindow(
  points: readonly TelemetryPoint[],
  nowMs: number,
  spanMs: number,
  buckets = 60,
): Aggregated[] {
  const n = Math.max(1, Math.min(MAX_POINTS, Math.floor(buckets)));
  const span = Math.max(1, spanMs);
  const start = nowMs - span;
  const width = span / n;
  const out: Aggregated[] = Array.from({ length: n }, (_, i) => ({
    t_ms: Math.round(start + i * width),
    tokens_out: 0,
    tokens_in: 0,
    tps: 0,
    cost_usd: 0,
    samples: 0,
  }));
  for (const p of points) {
    if (!Number.isFinite(p.t_ms) || p.t_ms < start || p.t_ms > nowMs) continue;
    const idx = Math.min(n - 1, Math.floor((p.t_ms - start) / width));
    const b = out[idx];
    b.tokens_out += p.tokens_out || 0;
    b.tokens_in += p.tokens_in || 0;
    b.cost_usd += p.cost_usd || 0;
    b.tps += p.tps || 0;
    b.samples += 1;
  }
  for (const b of out) if (b.samples > 0) b.tps = b.tps / b.samples;
  return out;
}

/** Totals of an aggregated window, for the stat tiles above the sparkline. */
export function windowTotals(buckets: readonly Aggregated[]): {
  tokens_out: number;
  tokens_in: number;
  cost_usd: number;
  tps_peak: number;
} {
  let tokens_out = 0;
  let tokens_in = 0;
  let cost_usd = 0;
  let tps_peak = 0;
  for (const b of buckets) {
    tokens_out += b.tokens_out;
    tokens_in += b.tokens_in;
    cost_usd += b.cost_usd;
    if (b.tps > tps_peak) tps_peak = b.tps;
  }
  return { tokens_out, tokens_in, cost_usd, tps_peak };
}

export type GuardianBand = "ok" | "warn" | "critical";

/** Ratio 0..1 of the window already held by the conversation. */
export function contextRatio(used: number, window: number): number {
  if (!Number.isFinite(window) || window <= 0) return 0;
  const r = (Number.isFinite(used) ? used : 0) / window;
  return r < 0 ? 0 : r > 1 ? 1 : r;
}

/**
 * The three Context Guardian bands. Thresholds are inclusive at the bottom:
 * < 70% ok, 70–90% warn, > 90% critical (the band that wears the error tone).
 */
export function guardianBand(ratio: number): GuardianBand {
  if (!Number.isFinite(ratio) || ratio < 0.7) return "ok";
  if (ratio <= 0.9) return "warn";
  return "critical";
}

/** Cache hit rate over the tokens the provider reported, 0..1. */
export function cacheHitRate(a: Pick<AgentTelemetry, "cache_read_tokens" | "input_tokens">): number {
  const total = (a.input_tokens || 0) + (a.cache_read_tokens || 0);
  if (total <= 0) return 0;
  return (a.cache_read_tokens || 0) / total;
}

/** Merge a new sample list into a ring, keeping it sorted and bounded. */
function pushHistory(
  ring: readonly TelemetryPoint[],
  incoming: readonly TelemetryPoint[],
  maxPoints = 2880,
): TelemetryPoint[] {
  if (incoming.length === 0) return ring as TelemetryPoint[];
  const byTs = new Map<number, TelemetryPoint>();
  for (const p of ring) byTs.set(p.t_ms, p);
  for (const p of incoming) if (Number.isFinite(p.t_ms)) byTs.set(p.t_ms, p);
  const merged = [...byTs.values()].sort((a, b) => a.t_ms - b.t_ms);
  return merged.length > maxPoints ? merged.slice(merged.length - maxPoints) : merged;
}

// ---------------------------------------------------------------------------
// Deterministic fake (tests + screenshots). Never shown without `?demo=1`.
// ---------------------------------------------------------------------------

/** xorshift32: same seed, same series, on every machine. */
function rng(seed: number): () => number {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 0x1_0000_0000;
  };
}

const FAKE_AGENTS: Array<
  Pick<AgentTelemetry, "agent_id" | "agent_name" | "provider" | "model" | "state"> & {
    window: number;
    fill: number;
    seed: number;
  }
> = [
  { agent_id: "coordinator", agent_name: "Nexus Coordinator", provider: "anthropic", model: "claude-sonnet-4-6", state: "streaming", window: 200_000, fill: 0.42, seed: 7 },
  { agent_id: "researcher", agent_name: "Team Advisor", provider: "openrouter", model: "gpt-5.2-mini", state: "waiting_tool", window: 128_000, fill: 0.81, seed: 21 },
  { agent_id: "writer", agent_name: "Chief of Staff", provider: "ollama", model: "qwen3:8b", state: "error", window: 32_768, fill: 0.94, seed: 44 },
];

/**
 * A snapshot with 24 h of plausible history. Deterministic given `nowMs`
 * rounded to the sample grid, so two screenshots of the same minute match.
 */
export function fakeSnapshot(nowMs: number): TelemetrySnapshot {
  const STEP = 300_000; // one sample per 5 min → 288 samples per 24 h
  const base = Math.floor(nowMs / STEP) * STEP;
  const agents: AgentTelemetry[] = FAKE_AGENTS.map((a, ai) => {
    const rand = rng(a.seed);
    const history: TelemetryPoint[] = [];
    let cost = 0;
    let out = 0;
    let inp = 0;
    for (let i = 287; i >= 0; i--) {
      const t = base - i * STEP;
      // Busy in bursts: three working stretches per day, quiet in between.
      const hour = ((t / 3_600_000) % 24 + 24) % 24;
      const busy = hour > 9 && hour < 12 ? 1 : hour > 14 && hour < 18 ? 0.8 : hour > 20 && hour < 22 ? 0.5 : 0.08;
      const active = rand() < busy;
      const tps = active ? 18 + rand() * 46 : 0;
      const tokens_out = Math.round(tps * 12);
      const tokens_in = Math.round(tokens_out * (1.4 + rand()));
      const c = (tokens_in * 3 + tokens_out * 15) / 1_000_000;
      out += tokens_out;
      inp += tokens_in;
      cost += c;
      history.push({ t_ms: t, tokens_out, tokens_in, tps: Math.round(tps * 10) / 10, cost_usd: c });
    }
    const events: AgentEvent[] = [
      { t_ms: base - 5_400_000, kind: "turn_started", detail: a.model },
      { t_ms: base - 5_100_000, kind: "rerouted", code: "ERR_LLM_RATE", detail: ai === 1 ? "openrouter/gpt-5.2-mini" : "anthropic/claude-haiku-4-6" },
      { t_ms: base - 3_600_000, kind: "turn_finished", detail: "12.4s" },
      ...(a.state === "error"
        ? [{ t_ms: base - 600_000, kind: "error" as const, code: "ERR_LLM_CONTEXT", detail: a.model }]
        : []),
    ];
    return {
      agent_id: a.agent_id,
      agent_name: a.agent_name,
      provider: a.provider,
      model: a.model,
      state: a.state,
      turns: 12 + ai * 7,
      errors: a.state === "error" ? 3 : 0,
      input_tokens: inp,
      output_tokens: out,
      cache_read_tokens: Math.round(inp * (0.55 - ai * 0.18)),
      cache_write_tokens: Math.round(inp * 0.04),
      first_token_ms_p50: [430, 820, 1610][ai],
      first_token_ms_last: [389, 913, 1744][ai],
      tps_last: [58.2, 31.4, 12.7][ai],
      cost_usd: cost,
      context_used_tokens: Math.round(a.window * a.fill),
      context_window: a.window,
      context_estimated: ai === 2,
      last_error_code: a.state === "error" ? "ERR_LLM_CONTEXT" : null,
      request_id: a.state === "streaming" ? "req-7f3a2c" : null,
      history,
      events,
    };
  });
  const cost_by_day: CostBucket[] = [];
  const dayRand = rng(99);
  for (let i = 13; i >= 0; i--) {
    const d = new Date(base - i * 86_400_000);
    cost_by_day.push({
      day: d.toISOString().slice(0, 10),
      cost_usd: Math.round(dayRand() * 320) / 100,
      calls: 4 + Math.round(dayRand() * 40),
    });
  }
  const quotas: QuotaStatus[] = [
    { id: "claude-max-1", label: "Claude Max · personal", kind: "cli", used: 0.62, resets_at_ms: base + 4_200_000, source: "reported", exhausts_at_ms: base + 9_000_000, active: true },
    { id: "claude-max-2", label: "Claude Max · work", kind: "cli", used: 0.18, resets_at_ms: base + 15_600_000, source: "reported", exhausts_at_ms: null, active: false },
    { id: "openrouter", label: "OpenRouter", kind: "byok", used: 0.93, resets_at_ms: null, source: "estimated", exhausts_at_ms: base + 1_500_000, active: false },
    { id: "ollama", label: "Ollama (local)", kind: "byok", used: 0, resets_at_ms: null, source: "reported", exhausts_at_ms: null, active: false },
  ];
  return {
    ts_ms: base,
    agents,
    quotas,
    cost_by_day,
    fallback_chain: ["claude-max-1", "claude-max-2", "openrouter", "ollama"],
  };
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export type LoadState = "idle" | "loading" | "ready" | "unavailable";

const EMPTY: TelemetrySnapshot = { ts_ms: 0, agents: [], quotas: [], cost_by_day: [], fallback_chain: [] };

let snapshot = $state<TelemetrySnapshot>(EMPTY);
let loadState = $state<LoadState>("idle");
let errorCode = $state("");
let demo = $state(false);
let unlisten: UnlistenFn | null = null;

export function getTelemetry(): TelemetrySnapshot {
  return snapshot;
}

export function getTelemetryState(): LoadState {
  return loadState;
}

export function getTelemetryError(): string {
  return errorCode;
}

export function isDemo(): boolean {
  return demo;
}

/** Normalize whatever the backend sends; a missing field never crashes a chart. */
function normalize(raw: unknown, previous: TelemetrySnapshot): TelemetrySnapshot {
  const r = (raw ?? {}) as Partial<TelemetrySnapshot>;
  const prevById = new Map(previous.agents.map((a) => [a.agent_id, a]));
  const agents = (r.agents ?? []).map((a) => {
    const prev = prevById.get(a.agent_id);
    return {
      ...a,
      history: pushHistory(prev?.history ?? [], a.history ?? []),
      events: a.events ?? [],
    } as AgentTelemetry;
  });
  return {
    ts_ms: r.ts_ms ?? Date.now(),
    agents,
    quotas: r.quotas ?? [],
    cost_by_day: r.cost_by_day ?? [],
    fallback_chain: r.fallback_chain ?? [],
  };
}

/** One-shot read. Also the refresh button. */
export async function loadTelemetry(): Promise<void> {
  if (demo) return;
  loadState = loadState === "ready" ? "ready" : "loading";
  try {
    const raw = await invoke<TelemetrySnapshot>("llm_telemetry_snapshot");
    snapshot = normalize(raw, snapshot);
    loadState = "ready";
    errorCode = "";
  } catch (e) {
    errorCode = String(e);
    if (loadState !== "ready") {
      snapshot = EMPTY;
      loadState = "unavailable";
    }
  }
}

/**
 * Cost per day from the ledger that already exists (`tool_usage_report`), used
 * when the snapshot brings none — every AI call in the app writes there, so
 * the chart has history from day one. Never called on its own timer.
 */
export async function loadCostFallback(days = 14): Promise<void> {
  if (demo || snapshot.cost_by_day.length > 0) return;
  try {
    const report = await invoke<{ by_day: Array<{ key: string; cost_usd: number; calls: number }> }>(
      "tool_usage_report",
      { days },
    );
    const by_day = report?.by_day ?? [];
    if (by_day.length === 0) return;
    snapshot = {
      ...snapshot,
      cost_by_day: by_day.map((b) => ({ day: b.key, cost_usd: b.cost_usd ?? 0, calls: b.calls ?? 0 })),
    };
  } catch {
    // The ledger is optional; an empty chart is a valid state.
  }
}

/** Per-agent reset: drop the local history so the charts restart clean. */
export function resetAgentHistory(agentId: string): void {
  snapshot = {
    ...snapshot,
    agents: snapshot.agents.map((a) => (a.agent_id === agentId ? { ...a, history: [], events: [] } : a)),
  };
}

/**
 * Subscribe to `llm://telemetry` and pull the first snapshot. The backend
 * pushes at 2 Hz while a turn is alive and stops in idle, so there is no timer
 * on this side. Returns the cleanup the page calls on destroy.
 */
export async function startTelemetry(options: { demo?: boolean } = {}): Promise<() => void> {
  demo = options.demo === true;
  if (demo) {
    snapshot = fakeSnapshot(Date.now());
    loadState = "ready";
    return () => stopTelemetry();
  }
  await loadTelemetry();
  try {
    unlisten = await listen<TelemetrySnapshot>("llm://telemetry", (ev) => {
      snapshot = normalize(ev.payload, snapshot);
      loadState = "ready";
    });
  } catch {
    // No event bridge (browser, tests): the one-shot read is all we get.
  }
  return () => stopTelemetry();
}

function stopTelemetry(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
}

