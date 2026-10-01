/**
 * Texts of the agents' OS notifications ("needs you", "a question went
 * unanswered", "finished", "failed").
 *
 * The notifications are sent by Rust (`llm_attention.rs`), which has no
 * locale, so — like the tray (`$lib/tray-strings`) and the seeded prompts
 * (`$lib/agent-prompts`) — the frontend resolves the text and pushes it
 * through `sync_llm_notify_strings`; the compiled-in English set is only the
 * fallback until the first push.
 *
 * Values come from `rawTranslations`, not `$t`: the backend fills
 * `{{agent}}`, `{{tool}}`, `{{preview}}` and `{{minutes}}` itself, and the
 * default parser would strip them.
 */
import type { TranslationBag } from "./agent-prompts";

// A type alias (not an interface): Tauri's `invoke` wants an index signature.
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

/** The payload of `sync_llm_notify_strings`: active locale, then English, then the key. */
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
