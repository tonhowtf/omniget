import { NAV_PARTS } from "./settings-nav";
import type { SettingAlias } from "./settings-nav/types";

export type SettingsCategory = "general" | "appearance" | "downloads" | "ai" | "integrations" | "advanced";

export const SETTINGS_CATEGORIES: readonly SettingsCategory[] = ["general", "appearance", "downloads", "ai", "integrations", "advanced"];
const DEFAULT_SETTINGS_CATEGORY: SettingsCategory = "downloads";

const SETTINGS_SECTIONS = Object.fromEntries(SETTINGS_CATEGORIES.map((c) => [c, NAV_PARTS[c].sections])) as Record<SettingsCategory, readonly string[]>;

export const CATEGORY_LABEL_KEYS: Record<SettingsCategory, string> = {
  general: "settings.rebrand.category.general",
  appearance: "settings.rebrand.category.appearance",
  downloads: "settings.rebrand.category.downloads",
  ai: "settings.rebrand.category.ai",
  integrations: "settings.rebrand.category.integrations",
  advanced: "settings.rebrand.category.advanced",
};

export const SECTION_LABEL_KEYS: Record<string, string> = Object.assign({}, ...SETTINGS_CATEGORIES.map((c) => NAV_PARTS[c].sectionLabels));

export const SETTING_ALIASES: Record<string, SettingAlias> = Object.assign({}, ...SETTINGS_CATEGORIES.map((c) => NAV_PARTS[c].aliases));

const TAB_SECTION_ALIASES: Record<string, [SettingsCategory, string | null]> = Object.assign({}, ...SETTINGS_CATEGORIES.map((c) => NAV_PARTS[c].legacy));


function isCategory(value: string | null): value is SettingsCategory {
  return value !== null && (SETTINGS_CATEGORIES as readonly string[]).includes(value);
}

function isSection(tab: SettingsCategory, section: string | null): section is string {
  return section !== null && SETTINGS_SECTIONS[tab].includes(section);
}

function sectionFromHash(hash: string): string | null {
  if (!hash || hash === "#") return null;
  const raw = decodeURIComponent(hash.slice(1));
  if (raw === "settings-pet") return "pet";
  const prefixed = /^settings-section-([a-z0-9-]+)$/.exec(raw);
  if (prefixed) return prefixed[1];
  return /^[a-z][a-z0-9-]*$/.test(raw) ? raw : null;
}

type SettingsLocation = {
  tab: SettingsCategory;
  section: string | null;
  setting: string | null;
  hash: string;
  canonicalHref: string;
  changed: boolean;
};

function withParams(pathname: string, first: [string, string][], rest: URLSearchParams, hash: string): string {
  const out = new URLSearchParams();
  for (const [key, value] of first) out.set(key, value);
  for (const [key, value] of rest) if (!out.has(key)) out.append(key, value);
  const search = out.toString();
  return `${pathname}${search ? `?${search}` : ""}${hash}`;
}

export function resolveSettingsLocation(url: URL): SettingsLocation {
  const params = url.searchParams;
  const rawTab = params.get("tab");
  const rawSection = params.get("section") || null;
  const setting = params.get("setting") || null;
  const hash = url.hash;
  const rest = new URLSearchParams(params);
  rest.delete("tab");
  rest.delete("section");
  rest.delete("setting");

  let tab: SettingsCategory | null = null;
  let section: string | null = null;
  let external: string | null = null;

  const alias = setting ? SETTING_ALIASES[setting] : undefined;
  if (alias && "href" in alias) {
    external = alias.href;
  } else if (alias) {
    tab = alias.tab;
    section = alias.section;
  }

  if (!tab && !external) {
    const oldTab = rawTab ?? "";
    const requested = rawSection ?? sectionFromHash(hash) ?? "";
    const pair = TAB_SECTION_ALIASES[`${oldTab}|${requested}`];
    if (pair) {
      [tab, section] = pair;
    } else if (isCategory(rawTab)) {
      tab = rawTab;
      section = isSection(rawTab, requested || null) ? requested : null;
    } else {
      const tabOnly = TAB_SECTION_ALIASES[`${oldTab}|`];
      if (tabOnly) [tab, section] = tabOnly;
      else tab = DEFAULT_SETTINGS_CATEGORY;
    }
  }

  let canonicalHref: string;
  if (external) {
    const target = new URL(external, url.origin);
    for (const [key, value] of rest) if (!target.searchParams.has(key)) target.searchParams.append(key, value);
    canonicalHref = `${target.pathname}?${target.searchParams}${hash}`;
    tab = "ai";
  } else {
    const first: [string, string][] = [["tab", tab!]];
    if (section) first.push(["section", section]);
    if (setting) first.push(["setting", setting]);
    canonicalHref = withParams(url.pathname, first, rest, hash);
  }
  const current = `${url.pathname}${url.search}${hash}`;
  return { tab: tab!, section, setting, hash, canonicalHref, changed: canonicalHref !== current };
}

export function settingsHref(tab: SettingsCategory, section?: string | null, setting?: string | null): string {
  const params = new URLSearchParams({ tab });
  if (section) params.set("section", section);
  if (setting) params.set("setting", setting);
  return `/settings?${params}`;
}

function oneOf<T extends string>(value: string | null, allowed: readonly T[]): T | null {
  return value !== null && (allowed as readonly string[]).includes(value) ? (value as T) : null;
}

export function parseMusicTarget(params: URLSearchParams): { view: "settings" | null; panel: "eq" | null } {
  const view = oneOf(params.get("view"), ["settings"] as const);
  return { view, panel: oneOf(params.get("panel"), ["eq"] as const) };
}

export function parseHomeTarget(params: URLSearchParams): { panel: "download-advanced" | null } {
  return { panel: oneOf(params.get("panel"), ["download-advanced"] as const) };
}

function prefersReducedMotion(): boolean {
  if (typeof document !== "undefined" && document.querySelector("[data-reduce-motion=\"true\"]")) return true;
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

const FOCUSABLE = "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])";

type FocusWhenReadyOptions = {
  timeoutMs?: number;
  preferControl?: boolean;
  onUnavailable?: () => void;
};

let initialNavigationPending = false;
let holdTimer: ReturnType<typeof setTimeout> | null = null;
const navigationWaiters = new Set<() => void>();

export function holdFocusUntilNavigated(maxMs = 3000): void {
  if (initialNavigationPending) return;
  initialNavigationPending = true;
  holdTimer = setTimeout(releaseFocusAfterNavigation, maxMs);
}

export function releaseFocusAfterNavigation(): void {
  if (!initialNavigationPending) return;
  initialNavigationPending = false;
  if (holdTimer) clearTimeout(holdTimer);
  holdTimer = null;
  const waiters = [...navigationWaiters];
  navigationWaiters.clear();
  for (const run of waiters) run();
}

const SETTLE_MS = 2000;
const INTERACTION_EVENTS = ["wheel", "touchstart", "pointerdown", "keydown"] as const;

function scrollParent(node: HTMLElement): HTMLElement | null {
  let current = node.parentElement;
  while (current && current !== document.body) {
    const overflow = getComputedStyle(current).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && current.scrollHeight > current.clientHeight) return current;
    current = current.parentElement;
  }
  return null;
}

function outOfView(target: HTMLElement): boolean {
  const rect = target.getBoundingClientRect();
  let top = 0;
  let bottom = window.innerHeight;
  const scroller = scrollParent(target);
  if (scroller) {
    const box = scroller.getBoundingClientRect();
    top = Math.max(top, box.top);
    bottom = Math.min(bottom, box.bottom);
  }
  return rect.bottom <= top || rect.top >= bottom - 32;
}

function keepInView(target: HTMLElement, focused: HTMLElement): () => void {
  if (typeof ResizeObserver === "undefined" || typeof document.addEventListener !== "function") return () => {};
  let stopped = false;
  let frame = 0;
  const boxes: HTMLElement[] = [];
  for (let node = target.parentElement; node && node !== document.body && boxes.length < 8; node = node.parentElement) boxes.push(node);
  const check = () => {
    frame = 0;
    if (stopped) return;
    if (document.activeElement !== focused) { stop(); return; }
    if (outOfView(target)) target.scrollIntoView({ block: "nearest", behavior: prefersReducedMotion() ? "auto" : "smooth" });
  };
  const observer = new ResizeObserver(() => {
    if (!frame) frame = requestAnimationFrame(check);
  });
  const stop = () => {
    if (stopped) return;
    stopped = true;
    observer.disconnect();
    if (frame) cancelAnimationFrame(frame);
    clearTimeout(timer);
    for (const type of INTERACTION_EVENTS) document.removeEventListener(type, stop, true);
  };
  for (const box of boxes) observer.observe(box);
  for (const type of INTERACTION_EVENTS) document.addEventListener(type, stop, { capture: true, passive: true });
  const timer = setTimeout(stop, SETTLE_MS);
  return stop;
}

export function focusWhenReady(selector: string, opts: FocusWhenReadyOptions = {}): () => void {
  if (typeof document === "undefined") return () => {};
  const timeoutMs = opts.timeoutMs ?? 4000;
  let done = false;
  let observer: MutationObserver | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let stopSettle: () => void = () => {};

  const finish = () => {
    done = true;
    navigationWaiters.delete(start);
    observer?.disconnect();
    observer = null;
    if (timer) clearTimeout(timer);
    timer = null;
  };

  const attempt = (): boolean => {
    if (done) return true;
    const target = document.querySelector<HTMLElement>(selector);
    if (!target) return false;
    if (revealAncestors(target)) return false;
    let focusTarget: HTMLElement;
    if (opts.preferControl) {
      const control = target.matches(FOCUSABLE) ? target : target.querySelector<HTMLElement>(FOCUSABLE);
      if (!control) {
        finish();
        opts.onUnavailable?.();
        return true;
      }
      focusTarget = control;
    } else {
      const region = target.querySelector<HTMLElement>(":scope > [data-disclosure-region][hidden]");
      if (region) {
        region.dispatchEvent(new CustomEvent("settings-reveal", { bubbles: false }));
        return false;
      }
      focusTarget = target;
      if (!target.matches(FOCUSABLE) && !target.hasAttribute("tabindex")) target.tabIndex = -1;
    }
    focusTarget.focus({ preventScroll: true });
    target.scrollIntoView({ block: "nearest", behavior: prefersReducedMotion() ? "auto" : "smooth" });
    finish();
    stopSettle = keepInView(target, focusTarget);
    return true;
  };

  function start() {
    if (done || attempt()) return;
    observer = new MutationObserver(() => { attempt(); });
    observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["hidden"] });
    timer = setTimeout(() => {
      if (done) return;
      finish();
      opts.onUnavailable?.();
    }, timeoutMs);
  }

  if (initialNavigationPending) navigationWaiters.add(start);
  else start();
  return () => { finish(); stopSettle(); };
}

function revealAncestors(node: HTMLElement): boolean {
  let revealed = false;
  let current: HTMLElement | null = node;
  while (current) {
    if (current.hidden && current.dataset.disclosureRegion !== undefined) {
      current.dispatchEvent(new CustomEvent("settings-reveal", { bubbles: false }));
      revealed = true;
    }
    current = current.parentElement;
  }
  return revealed;
}
