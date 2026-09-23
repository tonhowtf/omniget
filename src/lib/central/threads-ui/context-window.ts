// Context meter data: tokens in the model's window after the last request
// and the window size. The driver's `usedTokens/maxTokens` win when a live
// usage event carried them; otherwise we estimate from the last turn's usage
// and a table of known windows.

import type { UsageRow } from "$lib/central/threads/types";

const WINDOWS: [RegExp, number][] = [
  [/\[1m\]|-1m\b|1m-context/i, 1_000_000],
  [/gemini[-\w.]*(pro|flash)/i, 1_048_576],
  [/gpt-5|codex/i, 272_000],
  [/gpt-4\.1/i, 1_047_576],
  [/gpt-4o|o3|o4/i, 200_000],
  [/claude|sonnet|opus|haiku/i, 200_000],
  [/grok-4/i, 256_000],
  [/grok/i, 131_072],
  [/qwen3?-coder/i, 262_144],
  [/kimi|moonshot/i, 262_144],
  [/deepseek/i, 128_000],
  [/llama|mistral|gemma|phi/i, 128_000],
];

export function windowForModel(model: string | null | undefined, driver?: string | null): number | null {
  const key = `${model ?? ""} ${driver ?? ""}`;
  for (const [re, n] of WINDOWS) if (re.test(key)) return n;
  if (driver === "claude") return 200_000;
  if (driver === "codex") return 272_000;
  if (driver === "gemini") return 1_048_576;
  return null;
}

export type ContextFill = { used: number; max: number | null; estimated: boolean };

/** Prompt side of the last request ≈ what sits in the context now. */
export function contextFromUsage(u: UsageRow | null, model: string | null, driver: string | null): ContextFill | null {
  if (!u) return null;
  const used = u.inputTokens + u.cachedInputTokens + u.cacheWriteTokens + u.outputTokens;
  if (used <= 0) return null;
  return { used, max: windowForModel(u.model ?? model, driver), estimated: true };
}
