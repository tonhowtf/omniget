// Video summaries (/llm/summaries): thin wrappers over the existing AI commands,
// kept apart from the components so the call contract can be tested with a fake IPC.
import type { AiInvoke } from "$components/settings/settings-ai-form";

export type SummaryStyle = "short" | "balanced" | "detailed";

export type SummaryHistoryEntry = {
  id: number;
  kind: string;
  url: string;
  title: string;
  content: string;
  created_at_ms: number;
};

/** Runs one summary with the saved connection. Only called on an explicit action. */
export async function runSummary(invoke: AiInvoke, input: { url: string; style: SummaryStyle; lang: string }): Promise<void> {
  const url = input.url.trim();
  if (!url) throw new Error("empty_url");
  await invoke("ai_summarize_url", {
    url,
    style: input.style === "balanced" ? null : input.style,
    lang: input.lang.trim() || null,
  });
}

/** Newest first. Local read only. */
export async function loadSummaryHistory(invoke: AiInvoke): Promise<SummaryHistoryEntry[]> {
  const list = await invoke<SummaryHistoryEntry[]>("ai_history_list");
  return Array.isArray(list) ? list.slice().reverse() : [];
}

export function clearSummaryHistory(invoke: AiInvoke): Promise<unknown> {
  return invoke("ai_history_clear");
}

/** Known backend codes become copy; anything else is shown as given. */
export function summaryErrorKey(error: unknown): string | null {
  if (error === "no_transcript") return "settings.ai.no_transcript";
  if (error === "ai_not_configured") return "settings.ai.not_configured";
  return null;
}

/**
 * Which element a deep link reveals: an exact known setting key wins, then the
 * section, then nothing. Unknown values never select anything.
 */
export const SUMMARY_SETTING_KEYS: Record<string, "form" | "history"> = {
  "settings.ai.summarize_title": "form",
  "settings.ai.history": "history",
};

export function summaryFocusSelector(target: { section: "form" | "history" | null; setting: string | null }): string | null {
  if (target.setting && target.setting in SUMMARY_SETTING_KEYS) return `[data-setting-key="${target.setting}"]`;
  if (target.section) return `#summaries-${target.section}`;
  return null;
}
