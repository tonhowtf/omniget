import { afterEach, expect, it } from "vitest";
import { get } from "svelte/store";
import { loadTranslations, locale, t } from "./index";

const languages = ["en", "pt", "ru", "el", "zh", "zh-TW", "ja", "it", "fr", "es", "fa", "lo"];
afterEach(() => locale.set("en"));

it.each(languages)("renders the model count through the real translator in %s", async (language) => {
  await loadTranslations(language);
  locale.set(language);
  for (const count of [1, 2, 12]) {
    const rendered = get(t)("llm.models.count", { count });
    expect(rendered).toContain(String(count));
    expect(rendered).not.toMatch(/[{}]/);
  }
});
