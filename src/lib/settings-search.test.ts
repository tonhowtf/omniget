import { describe, expect, it, vi, beforeEach } from "vitest";
import en from "./i18n/en.json";
import pt from "./i18n/pt.json";
import { SETTINGS_SEARCH_INDEX, parseSettingNumber, searchSettings, settingsEntryUrl } from "./settings-search";

const mocks = vi.hoisted(() => ({ update: vi.fn(), invoke: vi.fn(), toast: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("$lib/stores/settings-store.svelte", () => ({ updateSettings: mocks.update, getSettings: vi.fn() }));
vi.mock("$lib/stores/toast-store.svelte", () => ({ showToast: mocks.toast }));
vi.mock("$lib/i18n", () => ({
  t: { subscribe: (run: (value: (key: string) => string) => void) => { run(key => key); return () => {}; } },
  locale: { set: vi.fn() },
  loadTranslations: vi.fn(),
}));
import { changeNumber, saveSettings } from "../components/settings/settings-helpers";

function translator(dictionary: unknown) {
  return (key: string) => {
    const value = key.split(".").reduce<unknown>((node, part) => (
      node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined
    ), dictionary);
    return typeof value === "string" ? value : key;
  };
}

describe("settings search", () => {
  const translate = translator(en);

  it("finds conditional provider and proxy fields without mounting or enabling them", () => {
    expect(searchSettings("local endpoint", translate).some(entry => entry.key === "settings.ai.local_url")).toBe(true);
    expect(searchSettings("proxy password", translate).some(entry => entry.key === "settings.proxy.password")).toBe(true);
  });

  it("finds channels and rules independently of their card markup", () => {
    expect(searchSettings("channel", translate).some(entry => entry.category === "channels")).toBe(true);
    expect(searchSettings("rule", translate).some(entry => entry.section === "rules")).toBe(true);
  });

  it("includes the advanced League and Music sections", () => {
    expect(SETTINGS_SEARCH_INDEX.some(entry => entry.key === "league.settings_auto_accept" && entry.section === "league")).toBe(true);
    expect(SETTINGS_SEARCH_INDEX.some(entry => entry.key === "music.settings_enable" && entry.section === "music")).toBe(true);
  });

  it("uses localized words and ignores accents and case", () => {
    const translatePt = translator(pt);
    const query = translatePt("settings.appearance.language").toUpperCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "");
    expect(searchSettings(query, translatePt).some(entry => entry.key === "settings.appearance.language")).toBe(true);
  });

  it("returns no unrelated settings for an empty or unmatched search", () => {
    expect(searchSettings("  ", translate)).toEqual([]);
    expect(searchSettings("xqz-no-matching-setting", translate)).toEqual([]);
  });

  it("has one canonical translated destination for every entry", () => {
    expect(new Set(SETTINGS_SEARCH_INDEX.map(entry => entry.key)).size).toBe(SETTINGS_SEARCH_INDEX.length);
    for (const entry of SETTINGS_SEARCH_INDEX) {
      expect(translate(entry.key), entry.key).not.toBe(entry.key);
      expect(translate(entry.sectionKey), entry.sectionKey).not.toBe(entry.sectionKey);
      const url = new URL(settingsEntryUrl(entry), "https://omniget.invalid");
      expect(url.pathname).toBe("/settings");
      expect(url.searchParams.get("setting")).toBe(entry.key);
      expect(url.searchParams.get("tab")).toBe(entry.category);
      expect(url.searchParams.get("section") ?? "").toBe(entry.section);
    }
  });
});

describe("numeric settings and persistence feedback", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.update.mockResolvedValue({ advanced: { stagger_delay_ms: 0 } });
    mocks.invoke.mockResolvedValue(undefined);
  });

  it("accepts zero when allowed, but rejects blanks, fractions and out-of-range values", () => {
    expect(parseSettingNumber("0", "0", "2000")).toBe(0);
    expect(parseSettingNumber("0", "1", "20")).toBeNull();
    for (const invalid of ["", " ", "Infinity", "abc", "-1", "2001", "0.5"]) {
      expect(parseSettingNumber(invalid, "0", "2000")).toBeNull();
    }
  });

  function numericEvent(value: string, min: string, max: string, valid = true) {
    const reportValidity = vi.fn();
    return { event: { target: { value, min, max, validity: { valid }, reportValidity } } as unknown as Event, reportValidity };
  }

  it("persists zero through the settings event handler", async () => {
    const { event } = numericEvent("0", "0", "2000");
    await changeNumber("advanced", "stagger_delay_ms", event);
    expect(mocks.update).toHaveBeenCalledWith({ advanced: { stagger_delay_ms: 0 } });
  });

  it("keeps invalid native input out of persistence and requests validation feedback", async () => {
    const { event, reportValidity } = numericEvent("50", "1", "10", false);
    await changeNumber("advanced", "max_concurrent_downloads", event);
    expect(mocks.update).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(reportValidity).toHaveBeenCalledOnce();
  });

  it("reports failed persistence without exposing raw backend content", async () => {
    mocks.update.mockRejectedValue(new Error("private backend path or token"));
    expect(await saveSettings({ appearance: { theme: "dark" } })).toBeNull();
    expect(mocks.toast).toHaveBeenCalledWith("error", "settings.save_failed");
  });

  it("does not update the live download limit after a failed save", async () => {
    mocks.update.mockRejectedValue(new Error("unavailable"));
    const { event } = numericEvent("2", "1", "10");
    await changeNumber("advanced", "max_concurrent_downloads", event);
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.toast).toHaveBeenCalledOnce();
  });
});
