import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import navigationMap from "./settings-navigation.fixture.json";
import {
  SETTINGS_SECTIONS,
  SETTING_ALIASES,
  focusWhenReady,
  holdFocusUntilNavigated,
  releaseFocusAfterNavigation,
  parseHomeTarget,
  parseMcpTarget,
  parseMusicTarget,
  parseRosterTarget,
  parseSettingsSection,
  parseSummariesTarget,
  resolveSettingsLocation,
} from "./settings-navigation";

const BASE = "https://omniget.invalid";
const resolve = (href: string) => resolveSettingsLocation(new URL(href, BASE));
const params = (query: string) => new URLSearchParams(query);

describe("resolveSettingsLocation", () => {
  const settingsCases = navigationMap.acceptanceCases.filter(
    (item): item is { input: string; expected: string } => "expected" in item && item.input.startsWith("/settings"),
  );

  it.each(settingsCases)("canonicalizes $input", ({ input, expected }) => {
    const location = resolve(input);
    expect(location.canonicalHref).toBe(expected);
    expect(location.changed).toBe(input !== expected);
  });

  it("is idempotent: a canonical URL never asks for another replaceState", () => {
    for (const { expected } of settingsCases) {
      const location = resolve(expected);
      expect(location.canonicalHref).toBe(expected);
      expect(location.changed).toBe(false);
    }
  });

  it("resolves every known setting alias regardless of the old tab and section", () => {
    for (const alias of navigationMap.settingAliases) {
      const location = resolve(`${alias.existingHref.startsWith("/settings") ? alias.existingHref : `/settings?setting=${alias.setting}`}`);
      if (alias.managerId) {
        expect(location.canonicalHref).toBe(alias.targetHref);
        continue;
      }
      expect(location.tab, alias.setting).toBe(alias.targetTab);
      expect(location.section ?? "", alias.setting).toBe(alias.targetSection ?? "");
      expect(location.setting).toBe(alias.setting);
      expect(location.canonicalHref).toBe(alias.targetHref);
    }
  });

  it("maps every tab/section alias to its target", () => {
    for (const alias of navigationMap.tabSectionAliases) {
      const query = new URLSearchParams({ tab: alias.tab });
      if (alias.section) query.set("section", alias.section);
      const location = resolve(`/settings?${query}`);
      expect(location.canonicalHref, `${alias.tab}|${alias.section}`).toBe(alias.targetHref);
    }
  });

  it("resolves legacy hashes through the same aliases and preserves the hash", () => {
    for (const alias of navigationMap.legacyHashAliases) {
      const location = resolve(`/settings?tab=${alias.tab}${alias.hash}`);
      expect(location.tab, alias.hash).toBe(alias.targetTab);
      expect(location.section, alias.hash).toBe(alias.targetSection);
      expect(location.hash).toBe(alias.hash);
      expect(location.canonicalHref.endsWith(alias.hash)).toBe(true);
    }
    expect(resolve("/settings?tab=advanced#music").canonicalHref).toBe("/settings?tab=integrations&section=music#music");
  });

  it("lets the setting query win over a legacy hash", () => {
    const location = resolve("/settings?tab=advanced&setting=settings.proxy.host#settings-section-music");
    expect(location.tab).toBe("advanced");
    expect(location.section).toBe("network");
  });

  it("keeps unrelated query parameters and unknown hashes", () => {
    const location = resolve("/settings?tab=network&from=palette#whatever-anchor");
    expect(location.canonicalHref).toBe("/settings?tab=advanced&section=network&from=palette#whatever-anchor");
  });

  it("keeps an unknown setting key but drops an unknown section", () => {
    const location = resolve("/settings?tab=general&section=nope&setting=not.a.key");
    expect(location.tab).toBe("general");
    expect(location.section).toBeNull();
    expect(location.canonicalHref).toBe("/settings?tab=general&setting=not.a.key");
  });

  it("defaults to downloads when no tab is given", () => {
    expect(resolve("/settings").tab).toBe("downloads");
    expect(resolve("/settings").canonicalHref).toBe("/settings?tab=downloads");
    expect(resolve("/settings?tab=").tab).toBe("downloads");
  });

  it("accepts every section of the six categories as-is", () => {
    for (const [tab, sections] of Object.entries(SETTINGS_SECTIONS)) {
      for (const section of sections) {
        const href = `/settings?tab=${tab}&section=${section}`;
        expect(resolve(href).canonicalHref, href).toBe(href);
      }
    }
  });

  it("re-resolves when the query changes on the same route", () => {
    expect(resolve("/settings?tab=downloads&section=hotkeys").section).toBe("clipboard");
    expect(resolve("/settings?tab=downloads&section=quality").section).toBe("quality");
    expect(resolve("/settings?tab=general&section=usage-tray").section).toBe("usage-tray");
    expect(resolve("/settings?tab=general&section=limits-strip").section).toBe("limits-strip");
  });

  it("sends the moved summary blocks to /llm/summaries and preserves setting", () => {
    expect(resolve("/settings?tab=ai&setting=settings.ai.summarize_title").canonicalHref)
      .toBe("/llm/summaries?section=form&setting=settings.ai.summarize_title");
    const history = resolve("/settings?tab=ai&setting=settings.ai.history&from=x");
    expect(history.canonicalHref).toBe("/llm/summaries?section=history&setting=settings.ai.history&from=x");
    expect(history.changed).toBe(true);
  });

  it("covers every alias of the navigation map", () => {
    expect(Object.keys(SETTING_ALIASES).length).toBe(new Set(navigationMap.settingAliases.map(alias => alias.setting)).size);
  });
});

describe("destination parsers", () => {
  it("roster: section only with a bot, unknown values are null", () => {
    expect(parseRosterTarget(params(""))).toEqual({ bot: null, section: null });
    expect(parseRosterTarget(params("section=skills"))).toEqual({ bot: null, section: null });
    expect(parseRosterTarget(params("bot=abc&section=skills"))).toEqual({ bot: "abc", section: "skills" });
    expect(parseRosterTarget(params("bot=abc&section=learning"))).toEqual({ bot: "abc", section: "learning" });
    expect(parseRosterTarget(params("bot=abc&section=grants"))).toEqual({ bot: "abc", section: null });
    expect(parseRosterTarget(params("bot=%20%20"))).toEqual({ bot: null, section: null });
    // A missing bot is only a lookup key; resolving it is the destination's job and never creates one.
    expect(parseRosterTarget(params("bot=does-not-exist")).bot).toBe("does-not-exist");
  });

  it("mcp: client by default, server sections only in server mode", () => {
    expect(parseMcpTarget(params(""))).toEqual({ mode: "client", section: null });
    expect(parseMcpTarget(params("mode=server"))).toEqual({ mode: "server", section: null });
    expect(parseMcpTarget(params("mode=server&section=permissions"))).toEqual({ mode: "server", section: "permissions" });
    expect(parseMcpTarget(params("mode=server&section=media"))).toEqual({ mode: "server", section: "media" });
    expect(parseMcpTarget(params("mode=client&section=media"))).toEqual({ mode: "client", section: null });
    expect(parseMcpTarget(params("mode=bogus&section=media"))).toEqual({ mode: "client", section: null });
    expect(parseMcpTarget(params("half=server"))).toEqual({ mode: "client", section: null });
  });

  it("music: only view=settings and panel=eq", () => {
    expect(parseMusicTarget(params(""))).toEqual({ view: null, panel: null });
    expect(parseMusicTarget(params("view=settings&panel=eq"))).toEqual({ view: "settings", panel: "eq" });
    expect(parseMusicTarget(params("view=library&panel=queue"))).toEqual({ view: null, panel: null });
  });

  it("home: only panel=download-advanced", () => {
    expect(parseHomeTarget(params("panel=download-advanced"))).toEqual({ panel: "download-advanced" });
    expect(parseHomeTarget(params("panel=download"))).toEqual({ panel: null });
    expect(parseHomeTarget(params(""))).toEqual({ panel: null });
  });

  it("summaries: form|history and the setting is preserved", () => {
    expect(parseSummariesTarget(params("section=form&setting=settings.ai.summarize_title")))
      .toEqual({ section: "form", setting: "settings.ai.summarize_title" });
    expect(parseSummariesTarget(params("section=history&setting=settings.ai.history")))
      .toEqual({ section: "history", setting: "settings.ai.history" });
    expect(parseSummariesTarget(params("section=run"))).toEqual({ section: null, setting: null });
    expect(parseSummariesTarget(params(""))).toEqual({ section: null, setting: null });
  });

  it("settings section allowlist", () => {
    const allowed = ["limits-strip", "usage-tray", "profile"];
    expect(parseSettingsSection(params("section=usage-tray"), allowed)).toBe("usage-tray");
    expect(parseSettingsSection(params("section=limits-strip"), allowed)).toBe("limits-strip");
    expect(parseSettingsSection(params("section=__proto__"), allowed)).toBeNull();
    expect(parseSettingsSection(params(""), allowed)).toBeNull();
  });
});

describe("focusWhenReady", () => {
  type FakeNode = { hidden: boolean; dataset: Record<string, string>; parentElement: FakeNode | null; tabIndex: number;
    matches: (s: string) => boolean; hasAttribute: (n: string) => boolean; focus: ReturnType<typeof vi.fn>;
    querySelector: (s: string) => FakeNode | null;
    scrollIntoView: ReturnType<typeof vi.fn>; dispatchEvent: ReturnType<typeof vi.fn> };
  let present: FakeNode | null;
  let observerCallback: (() => void) | null;

  function node(extra: Partial<FakeNode> = {}): FakeNode {
    return { hidden: false, dataset: {}, parentElement: null, tabIndex: 0, matches: () => false, hasAttribute: () => false, querySelector: () => null,
      focus: vi.fn(), scrollIntoView: vi.fn(), dispatchEvent: vi.fn(), ...extra };
  }

  beforeEach(() => {
    vi.useFakeTimers();
    present = null;
    observerCallback = null;
    vi.stubGlobal("document", { querySelector: (selector: string) => (selector.startsWith("[data-reduce") ? null : present), body: {} });
    vi.stubGlobal("window", { matchMedia: () => ({ matches: true }) });
    vi.stubGlobal("MutationObserver", class { constructor(cb: () => void) { observerCallback = cb; } observe() {} disconnect() { observerCallback = null; } });
    vi.stubGlobal("CustomEvent", class { constructor(public type: string) {} });
  });
  afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

  it("waits for a late target, then focuses a section with tabindex=-1 without smooth scrolling", () => {
    focusWhenReady("#settings-group-usage-tray");
    expect(observerCallback).not.toBeNull();
    present = node();
    observerCallback?.();
    expect(present.focus).toHaveBeenCalledOnce();
    expect(present.tabIndex).toBe(-1);
    expect(present.scrollIntoView).toHaveBeenCalledWith({ block: "nearest", behavior: "auto" });
    expect(observerCallback).toBeNull();
  });

  it("opens a collapsed disclosure before focusing inside it", () => {
    const region = node({ hidden: true, dataset: { disclosureRegion: "" } });
    present = node({ parentElement: region, matches: () => true });
    focusWhenReady("#x");
    expect(region.dispatchEvent).toHaveBeenCalled();
    expect(present.focus).not.toHaveBeenCalled();
    region.hidden = false;
    observerCallback?.();
    expect(present.focus).toHaveBeenCalledOnce();
  });

  it("opens a collapsed section it was asked to show, then focuses it", () => {
    const region = node({ hidden: true, dataset: { disclosureRegion: "" } });
    present = node({ querySelector: () => (region.hidden ? region : null) });
    focusWhenReady("#settings-group-typography");
    expect(region.dispatchEvent).toHaveBeenCalledOnce();
    expect(present.focus).not.toHaveBeenCalled();
    region.hidden = false;
    observerCallback?.();
    expect(present.focus).toHaveBeenCalledOnce();
  });

  it("focuses the enabled control of a setting row, or falls back when it has none", () => {
    const control = node();
    present = node({ querySelector: () => control });
    focusWhenReady("[data-setting-key=x]", { preferControl: true });
    expect(control.focus).toHaveBeenCalledOnce();
    expect(present.focus).not.toHaveBeenCalled();

    const fallback = vi.fn();
    present = node({ querySelector: () => null });
    focusWhenReady("[data-setting-key=y]", { preferControl: true, onUnavailable: fallback });
    expect(fallback).toHaveBeenCalledOnce();
    expect(present.focus).not.toHaveBeenCalled();
  });

  it("on a cold load waits for the initial navigation before focusing (SvelteKit resets focus first)", () => {
    present = node();
    holdFocusUntilNavigated();
    focusWhenReady("#settings-group-usage-tray");
    expect(present.focus).not.toHaveBeenCalled();
    releaseFocusAfterNavigation();
    expect(present.focus).toHaveBeenCalledOnce();
    // Released once: later requests (client navigation) focus right away.
    const next = node();
    present = next;
    focusWhenReady("#settings-group-limits-strip");
    expect(next.focus).toHaveBeenCalledOnce();
  });

  it("a held request cleaned up before the navigation ends never focuses, and the hold expires", () => {
    present = node();
    holdFocusUntilNavigated(500);
    const cleanup = focusWhenReady("#settings-group-usage-tray");
    cleanup();
    vi.advanceTimersByTime(500);
    expect(present.focus).not.toHaveBeenCalled();
    focusWhenReady("#settings-group-usage-tray");
    expect(present.focus).toHaveBeenCalledOnce();
  });

  it("gives up after the timeout, reports it, and cleanup stops observing", () => {
    const fallback = vi.fn();
    const cleanup = focusWhenReady("#never", { timeoutMs: 100, onUnavailable: fallback });
    vi.advanceTimersByTime(100);
    expect(observerCallback).toBeNull();
    expect(fallback).toHaveBeenCalledOnce();
    cleanup();
  });
});
