import { describe, expect, it } from "vitest";
import { trayStrings, type TranslationBag } from "./tray-strings";

import el from "./i18n/el.json";
import en from "./i18n/en.json";
import es from "./i18n/es.json";
import fa from "./i18n/fa.json";
import fr from "./i18n/fr.json";
import itLocale from "./i18n/it.json";
import ja from "./i18n/ja.json";
import lo from "./i18n/lo.json";
import pt from "./i18n/pt.json";
import ru from "./i18n/ru.json";
import zhTW from "./i18n/zh-TW.json";
import zh from "./i18n/zh.json";

/** Mirrors what sveltekit-i18n stores: `{ "tray.quit": "Quit", … }` per locale. */
function flatten(value: unknown, prefix = "", out: Record<string, string> = {}): Record<string, string> {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      flatten(child, prefix ? `${prefix}.${key}` : key, out);
    }
  } else if (typeof value === "string") {
    out[prefix] = value;
  }
  return out;
}

const LOCALES: Array<[string, unknown]> = [
  ["el", el],
  ["en", en],
  ["es", es],
  ["fa", fa],
  ["fr", fr],
  ["it", itLocale],
  ["ja", ja],
  ["lo", lo],
  ["pt", pt],
  ["ru", ru],
  ["zh-TW", zhTW],
  ["zh", zh],
];

const BAG: TranslationBag = Object.fromEntries(LOCALES.map(([name, json]) => [name, flatten(json)]));

/**
 * These three strings are formatted by Rust on every tray update, so the
 * placeholders have to survive the trip. `$t()` would strip them: the default
 * parser replaces `{{count}}` with an empty payload value, which is why the tray
 * pushes raw values instead.
 */
const TEMPLATES: Array<[keyof ReturnType<typeof trayStrings>, string[]]> = [
  ["downloadsActive", ["{{count}}"]],
  ["tooltipActive", ["{{count}}"]],
  ["tooltipSpeed", ["{{count}}", "{{speed}}"]],
];

describe("trayStrings", () => {
  it("keeps the placeholders Rust fills, in every locale", () => {
    for (const [locale] of LOCALES) {
      const strings = trayStrings(BAG, locale);
      for (const [field, tokens] of TEMPLATES) {
        for (const token of tokens) {
          expect(strings[field], `${locale}.${field} lost ${token}`).toContain(token);
        }
      }
    }
  });

  it("sends the active locale, not the English fallback", () => {
    const strings = trayStrings(BAG, "ru");
    expect(strings.quit).toBe("Выход");
    expect(strings.downloadsActive).toBe("Активных загрузок: {{count}}");
    expect(strings.tooltipActive).toContain("{{count}}");
  });

  it("keeps the channels title on the same key sync_channels_tray sends", () => {
    const strings = trayStrings(BAG, "ru");
    expect(strings.channels).toBe((ru as { settings: { channels: { tray_header: string } } }).settings.channels.tray_header);
  });

  it("falls back to English for a locale that is not loaded yet", () => {
    const strings = trayStrings(BAG, "de");
    expect(strings.quit).toBe("Quit");
    expect(strings.tooltipSpeed).toBe("OmniGet — {{count}} active · {{speed}}");
  });

  it("falls back to the key itself when nothing is loaded", () => {
    const strings = trayStrings(undefined, "en");
    expect(strings.quit).toBe("tray.quit");
    expect(strings.downloadsActive).toBe("tray.downloads_active");
  });
});
