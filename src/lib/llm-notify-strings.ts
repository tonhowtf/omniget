import type { TranslationBag } from "./agent-prompts";

type LlmNotifyStrings = {
  needsYouTitle: string;
  needsYouBody: string;
  expiredTitle: string;
  expiredBody: string;
  finishedTitle: string;
  failedTitle: string;
};

const KEYS: Record<keyof LlmNotifyStrings, string> = {
  needsYouTitle: "llm.notify.needs_you_title",
  needsYouBody: "llm.notify.needs_you_body",
  expiredTitle: "llm.notify.expired_title",
  expiredBody: "llm.notify.expired_body",
  finishedTitle: "llm.notify.finished_title",
  failedTitle: "llm.notify.failed_title",
};

export function llmNotifyStrings(bag: TranslationBag, locale: string): LlmNotifyStrings {
  const pick = (key: string): string => {
    const active = bag?.[locale]?.[key];
    if (typeof active === "string" && active.length > 0) return active;
    const fallback = bag?.en?.[key];
    if (typeof fallback === "string" && fallback.length > 0) return fallback;
    return key;
  };
  const out = {} as LlmNotifyStrings;
  for (const [field, key] of Object.entries(KEYS) as [keyof LlmNotifyStrings, string][]) {
    out[field] = pick(key);
  }
  return out;
}
