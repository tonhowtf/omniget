import { describe, expect, it } from "vitest";
import { llmNotifyStrings } from "./llm-notify-strings";
import type { TranslationBag } from "./agent-prompts";

const EN: Record<string, string> = {
  "llm.notify.needs_you_title": "{{agent}} needs you",
  "llm.notify.needs_you_body": "Allow {{tool}}? {{preview}}",
  "llm.notify.expired_title": "{{agent}}: a question went unanswered",
  "llm.notify.expired_body": "Nobody answered {{tool}} in {{minutes}} min, so it did not run.",
  "llm.notify.finished_title": "{{agent}} finished",
  "llm.notify.failed_title": "{{agent}} failed",
};

describe("llmNotifyStrings", () => {
  it("keeps the placeholders the backend fills in", () => {
    const s = llmNotifyStrings({ en: EN }, "en");
    expect(s.needsYouTitle).toBe("{{agent}} needs you");
    expect(s.needsYouBody).toContain("{{tool}}");
    expect(s.needsYouBody).toContain("{{preview}}");
    expect(s.expiredBody).toContain("{{minutes}}");
  });

  it("prefers the active locale and falls back to English per key", () => {
    const bag: TranslationBag = {
      en: EN,
      pt: { "llm.notify.needs_you_title": "{{agent}} precisa de você" },
    };
    const s = llmNotifyStrings(bag, "pt");
    expect(s.needsYouTitle).toBe("{{agent}} precisa de você");
    expect(s.failedTitle).toBe("{{agent}} failed");
  });

  it("sends every field the backend reads", () => {
    const s = llmNotifyStrings({ en: EN }, "en");
    expect(Object.keys(s).sort()).toEqual(
      ["expiredBody", "expiredTitle", "failedTitle", "finishedTitle", "needsYouBody", "needsYouTitle"],
    );
  });
});
