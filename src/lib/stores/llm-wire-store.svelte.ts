/**
 * Wire probe store.
 *
 * Holds the evidence that a generation parameter reached the provider: the
 * redacted body sent with value A, the one sent with value B, both replies and
 * the verdict. Nothing here runs on its own — `estimateRound` and `runRound`
 * are only called from a click, and `loadLast` reads the round already in
 * memory on the Rust side without spending a request.
 */
import { invoke } from "@tauri-apps/api/core";

export type Verdict = "proven" | "sent" | "ignored" | "unsupported";

export interface ProbeResult {
  provider: string;
  model: string;
  param: string;
  /** Redacted request body sent with value A; `null` when the provider captured nothing. */
  sent_a: unknown;
  sent_b: unknown;
  reply_a: string;
  reply_b: string;
  differs: boolean;
  cost_usd: number;
  verdict: Verdict;
  detail: string;
}

interface ProbeRound {
  provider: string;
  model: string;
  started_at_ms: number;
  results: ProbeResult[];
  cost_usd: number;
}

interface ProbeEstimate {
  cases: number;
  requests: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number | null;
}

/** Same order the Rust side runs them in. `reasoning_effort` is the opt-in one. */
export const PROBE_PARAMS = [
  "temperature",
  "top_p",
  "max_tokens",
  "stop",
  "reasoning_effort",
] as const;
export type ProbeParam = (typeof PROBE_PARAMS)[number];

export const MAX_OUTPUT_TOKENS = 200;

// ---------------------------------------------------------------- pure helpers

/** i18n key for a verdict badge. */
export function verdictKey(v: Verdict): string {
  return `llm.wire.verdict.${v}`;
}

/** Colour role, so the panel does not hard-code a palette per verdict. */
export function verdictTone(v: Verdict): "ok" | "warn" | "bad" | "muted" {
  switch (v) {
    case "proven":
      return "ok";
    case "sent":
      return "warn";
    case "ignored":
      return "bad";
    default:
      return "muted";
  }
}

/** Maps a Rust error string (`ERR_LLM_*: message`) to an i18n key. */
function wireErrorKey(err: unknown): string {
  const text = typeof err === "string" ? err : String((err as Error)?.message ?? err ?? "");
  const code = text.split(":", 1)[0]?.trim() ?? "";
  switch (code) {
    case "ERR_LLM_MODEL":
      return "llm.wire.error.no_provider";
    case "ERR_LLM_BUDGET":
      return "llm.wire.error.budget";
    case "ERR_LLM_AUTH":
      return "llm.wire.error.auth";
    case "ERR_LLM_NET":
      return "llm.wire.error.net";
    case "ERR_LLM_STUB":
      return "llm.wire.error.stub";
    default:
      return "llm.wire.error.generic";
  }
}

/** `< $0.01` is a number too; only `null` means "price unknown". */
export function formatCost(usd: number | null | undefined): string {
  if (usd === null || usd === undefined) return "—";
  if (usd === 0) return "$0.00";
  if (usd < 0.01) return "< $0.01";
  return `$${usd.toFixed(2)}`;
}

/** Stable, indented JSON for the evidence blocks. `null` renders as a dash. */
export function prettyBody(body: unknown): string {
  if (body === null || body === undefined) return "—";
  try {
    return JSON.stringify(body, null, 2);
  } catch {
    return String(body);
  }
}

/**
 * Line-level diff between the two bodies, so the eye lands on the parameter
 * instead of reading two JSON blocks. `same` lines are shown dimmed.
 */
export interface DiffLine {
  kind: "same" | "a" | "b";
  text: string;
}

export function diffBodies(a: unknown, b: unknown): DiffLine[] {
  const left = prettyBody(a).split("\n");
  const right = prettyBody(b).split("\n");
  const out: DiffLine[] = [];
  const max = Math.max(left.length, right.length);
  for (let i = 0; i < max; i++) {
    const l = left[i];
    const r = right[i];
    if (l === r) {
      if (l !== undefined) out.push({ kind: "same", text: l });
      continue;
    }
    if (l !== undefined) out.push({ kind: "a", text: l });
    if (r !== undefined) out.push({ kind: "b", text: r });
  }
  return out;
}

/** How many results carry each verdict, for the one-line summary. */
export function tally(round: ProbeRound | null): Record<Verdict, number> {
  const out: Record<Verdict, number> = { proven: 0, sent: 0, ignored: 0, unsupported: 0 };
  for (const r of round?.results ?? []) out[r.verdict] = (out[r.verdict] ?? 0) + 1;
  return out;
}

// ---------------------------------------------------------------------- state

let round = $state<ProbeRound | null>(null);
let estimateState = $state<ProbeEstimate | null>(null);
let running = $state(false);
let loading = $state(false);
let error = $state<string | null>(null);
let loadedOnce = false;
let inFlight: Promise<void> | null = null;

export function getRound(): ProbeRound | null {
  return round;
}
export function getEstimate(): ProbeEstimate | null {
  return estimateState;
}
export function isRunning(): boolean {
  return running;
}
export function isLoading(): boolean {
  return loading;
}
/** i18n key, never a raw message. */
export function getWireError(): string | null {
  return error;
}

// -------------------------------------------------------------------- actions

/** Reads the round already in memory. Once per session, no polling. */
export async function loadLast(force = false): Promise<void> {
  if (inFlight) return inFlight;
  if (loadedOnce && !force) return;
  loading = true;
  inFlight = (async () => {
    try {
      const value = await invoke<ProbeRound | null>("llm_wire_probe_last");
      round = value ?? null;
      loadedOnce = true;
    } catch (e) {
      error = wireErrorKey(e);
    } finally {
      loading = false;
      inFlight = null;
    }
  })();
  return inFlight;
}

/** What the round would cost. No request leaves the machine. */
export async function estimateRound(
  provider: string,
  model: string,
  params?: ProbeParam[],
): Promise<ProbeEstimate | null> {
  try {
    estimateState = await invoke<ProbeEstimate>("llm_wire_probe_run", {
      provider,
      model,
      cases: params ?? null,
      estimateOnly: true,
    });
    return estimateState;
  } catch (e) {
    error = wireErrorKey(e);
    return null;
  }
}

/** Runs the round. Only from a click, and never twice at the same time. */
export async function runRound(
  provider: string,
  model: string,
  params?: ProbeParam[],
): Promise<boolean> {
  if (running) return false;
  running = true;
  error = null;
  try {
    round = await invoke<ProbeRound>("llm_wire_probe_run", {
      provider,
      model,
      cases: params ?? null,
      estimateOnly: false,
    });
    loadedOnce = true;
    return true;
  } catch (e) {
    error = wireErrorKey(e);
    return false;
  } finally {
    running = false;
  }
}
