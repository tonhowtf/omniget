// Settings destinations: one place for tab/section/setting parsing, legacy aliases and
// the allowlists other routes use to reveal a block. Parsers only read; unknown values
// resolve to null/defaults and never trigger an action.
// Source of truth: docs/settings-rebrand/navigation-map.json (closed planning contract).

export type SettingsCategory = "general" | "appearance" | "downloads" | "ai" | "integrations" | "advanced";

export const SETTINGS_CATEGORIES: readonly SettingsCategory[] = ["general", "appearance", "downloads", "ai", "integrations", "advanced"];
export const DEFAULT_SETTINGS_CATEGORY: SettingsCategory = "downloads";

/** Section ids per category, in render order. Each becomes `settings-group-<id>`. */
export const SETTINGS_SECTIONS: Record<SettingsCategory, readonly string[]> = {
  general: ["language", "startup", "profile", "guidance", "limits-strip", "usage-tray"],
  appearance: ["theme", "accessibility", "typography"],
  downloads: ["files", "quality", "presets", "subtitles", "clipboard", "cookies", "channels", "rules", "sites", "templates"],
  ai: ["connections", "managers", "execution"],
  integrations: ["extension", "music", "video", "tube", "read", "telegram", "washmybelly", "world", "canvas", "pet", "league", "rpc", "mcp-server"],
  advanced: ["network", "performance", "dependencies", "storage", "expert", "debug", "reset"],
};

export const CATEGORY_LABEL_KEYS: Record<SettingsCategory, string> = {
  general: "settings.rebrand.category.general",
  appearance: "settings.rebrand.category.appearance",
  downloads: "settings.rebrand.category.downloads",
  ai: "settings.rebrand.category.ai",
  integrations: "settings.rebrand.category.integrations",
  advanced: "settings.rebrand.category.advanced",
};

/** Heading key for each section id; menus, group titles and search share it. */
export const SECTION_LABEL_KEYS: Record<string, string> = {
  language: "settings.appearance.language",
  startup: "settings.general.title",
  profile: "profile.title",
  guidance: "settings.general.reset_hints",
  "limits-strip": "llm.limits.title",
  "usage-tray": "usage_tray.menu.toggle",
  theme: "settings.appearance.theme",
  accessibility: "settings.appearance.accessibility_title",
  typography: "settings.typography.section_title",
  files: "settings.rebrand.downloads.output",
  quality: "settings.rebrand.downloads.quality",
  presets: "settings.download.section_quality",
  subtitles: "settings.rebrand.downloads.subtitles",
  clipboard: "settings.download.section_clipboard_hotkeys",
  cookies: "settings.cookies.title",
  channels: "settings.channels.title",
  rules: "settings.rules.title",
  sites: "settings.download.section_per_platform",
  templates: "omnibox.adv.templates_title",
  connections: "settings.ai.title",
  managers: "llm.accounts.title",
  execution: "llm.watchdog.title",
  extension: "settings.browser_ext.title",
  music: "music.settings_title",
  video: "video.settings_title",
  tube: "tube.settings_title",
  read: "read.settings_title",
  telegram: "telegram.settings_title",
  washmybelly: "washmybelly.settings_title",
  world: "world.settings.title",
  canvas: "canvas.settings.title",
  pet: "pet.title",
  league: "league.settings_title",
  rpc: "settings.rpc.title",
  "mcp-server": "llm.mcp.server_title",
  network: "settings.cat_network",
  performance: "settings.advanced.section_performance",
  dependencies: "settings.dependencies.title",
  storage: "settings.storage.title",
  expert: "settings.advanced.section_expert",
  debug: "settings.advanced.section_debug",
  reset: "settings.advanced.reset",
};

type SettingAlias = { tab: SettingsCategory; section: string | null } | { href: string };

/** Exact translation-key aliases; they win over any old tab/section in the URL. */
export const SETTING_ALIASES: Record<string, SettingAlias> = {
  "design.title": { tab: "appearance", section: "theme" },
  "design.hint": { tab: "appearance", section: "theme" },
  "design.system": { tab: "appearance", section: "theme" },
  "design.light": { tab: "appearance", section: "theme" },
  "design.dark": { tab: "appearance", section: "theme" },
  "settings.cat_appearance": { tab: "appearance", section: null },
  "settings.general.start_with_system": { tab: "general", section: "startup" },
  "settings.general.start_minimized": { tab: "general", section: "startup" },
  "settings.general.exit_on_close": { tab: "general", section: "startup" },
  "settings.general.run_in_background": { tab: "general", section: "startup" },
  "settings.appearance.reduce_motion": { tab: "appearance", section: "accessibility" },
  "settings.appearance.reduce_transparency": { tab: "appearance", section: "accessibility" },
  "settings.appearance.theme": { tab: "appearance", section: "theme" },
  "settings.appearance.language": { tab: "general", section: "language" },
  "settings.general.reset_hints": { tab: "general", section: "guidance" },
  "settings.rpc.title": { tab: "integrations", section: "rpc" },
  "settings.rpc.enable": { tab: "integrations", section: "rpc" },
  "settings.rpc.test": { tab: "integrations", section: "rpc" },
  "settings.rpc.app_id": { tab: "integrations", section: "rpc" },
  "settings.rpc.large_image": { tab: "integrations", section: "rpc" },
  "profile.title": { tab: "general", section: "profile" },
  "profile.desc": { tab: "general", section: "profile" },
  "settings.typography.section_title": { tab: "appearance", section: "typography" },
  "settings.typography.presets_title": { tab: "appearance", section: "typography" },
  "settings.typography.fonts_title": { tab: "appearance", section: "typography" },
  "settings.typography.line_height_label": { tab: "appearance", section: "typography" },
  "settings.typography.spacing_label": { tab: "appearance", section: "typography" },
  "settings.typography.preview_title": { tab: "appearance", section: "typography" },
  "settings.typography.display_label": { tab: "appearance", section: "typography" },
  "settings.typography.body_label": { tab: "appearance", section: "typography" },
  "settings.typography.mono_label": { tab: "appearance", section: "typography" },
  "settings.download.section_output": { tab: "downloads", section: "files" },
  "settings.download.default_output_dir": { tab: "downloads", section: "files" },
  "settings.download.organize_by_platform": { tab: "downloads", section: "files" },
  "settings.download.skip_existing": { tab: "downloads", section: "files" },
  "settings.download.filename_template": { tab: "downloads", section: "files" },
  "settings.download.section_quality": { tab: "downloads", section: "quality" },
  "settings.download.video_quality": { tab: "downloads", section: "quality" },
  "settings.download.speed_limit": { tab: "downloads", section: "files" },
  "settings.download.always_ask_path": { tab: "downloads", section: "files" },
  "settings.download.section_subtitles": { tab: "downloads", section: "subtitles" },
  "settings.download.download_subtitles": { tab: "downloads", section: "subtitles" },
  "settings.download.include_auto_subtitles": { tab: "downloads", section: "subtitles" },
  "settings.download.caption_locale": { tab: "downloads", section: "subtitles" },
  "settings.download.keep_vtt": { tab: "downloads", section: "subtitles" },
  "settings.download.translate_metadata": { tab: "downloads", section: "files" },
  "settings.download.embed_metadata": { tab: "downloads", section: "files" },
  "settings.download.embed_thumbnail": { tab: "downloads", section: "files" },
  "settings.download.write_nfo_sidecar": { tab: "downloads", section: "files" },
  "settings.download.section_clipboard_hotkeys": { tab: "downloads", section: "clipboard" },
  "settings.download.clipboard_detection": { tab: "downloads", section: "clipboard" },
  "settings.download.auto_download_on_paste": { tab: "downloads", section: "clipboard" },
  "settings.download.hotkey_enabled": { tab: "downloads", section: "clipboard" },
  "settings.download.hotkey_binding": { tab: "downloads", section: "clipboard" },
  "settings.download.copy_to_clipboard_on_hotkey": { tab: "downloads", section: "clipboard" },
  "settings.download.music_hotkey_enabled": { tab: "downloads", section: "clipboard" },
  "settings.download.music_hotkey_binding": { tab: "downloads", section: "clipboard" },
  "settings.download.music_audio_format": { tab: "downloads", section: "files" },
  "settings.download.section_per_platform": { tab: "downloads", section: "sites" },
  "settings.download.youtube_sponsorblock": { tab: "downloads", section: "sites" },
  "settings.download.sb_mode": { tab: "downloads", section: "sites" },
  "settings.download.sb_categories": { tab: "downloads", section: "sites" },
  "settings.download.split_by_chapters": { tab: "downloads", section: "sites" },
  "settings.download.live_from_start": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_container_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_danmaku_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_danmaku_format_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_nfo_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_cover_sidecar_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_cover_format_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_naming_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_cdn_label": { tab: "downloads", section: "sites" },
  "settings.download.bilibili_cdn_prefer_label": { tab: "downloads", section: "sites" },
  "settings.bridge.title": { tab: "integrations", section: "extension" },
  "settings.bridge.pair_label": { tab: "integrations", section: "extension" },
  "settings.bridge.endpoint_label": { tab: "integrations", section: "extension" },
  "settings.bridge.token_label": { tab: "integrations", section: "extension" },
  "settings.bridge.rotate_label": { tab: "integrations", section: "extension" },
  "settings.proxy.title": { tab: "advanced", section: "network" },
  "settings.proxy.enabled": { tab: "advanced", section: "network" },
  "settings.proxy.type": { tab: "advanced", section: "network" },
  "settings.proxy.host": { tab: "advanced", section: "network" },
  "settings.proxy.port": { tab: "advanced", section: "network" },
  "settings.proxy.username": { tab: "advanced", section: "network" },
  "settings.proxy.password": { tab: "advanced", section: "network" },
  "settings.cookies.title": { tab: "downloads", section: "cookies" },
  "settings.cookies.managed_only_title": { tab: "downloads", section: "cookies" },
  "settings.cookies.capture_via_extension": { tab: "downloads", section: "cookies" },
  "settings.cookies.import_trigger": { tab: "downloads", section: "cookies" },
  "settings.cookies.action_test": { tab: "downloads", section: "cookies" },
  "settings.channels.title": { tab: "downloads", section: "channels" },
  "settings.channels.add": { tab: "downloads", section: "channels" },
  "settings.channels.auto_download": { tab: "downloads", section: "channels" },
  "settings.channels.interval": { tab: "downloads", section: "channels" },
  "settings.channels.check_now": { tab: "downloads", section: "channels" },
  "settings.ai.title": { tab: "ai", section: null },
  "settings.ai.provider": { tab: "ai", section: null },
  "settings.ai.model": { tab: "ai", section: null },
  "settings.ai.openai_key": { tab: "ai", section: null },
  "settings.ai.anthropic_key": { tab: "ai", section: null },
  "settings.ai.local_url": { tab: "ai", section: null },
  "settings.ai.test": { tab: "ai", section: null },
  "settings.ai.summarize_title": { href: "/llm/summaries?section=form&setting=settings.ai.summarize_title" },
  "settings.ai.history": { href: "/llm/summaries?section=history&setting=settings.ai.history" },
  "world.settings.title": { tab: "integrations", section: "world" },
  "world.settings.enabled": { tab: "integrations", section: "world" },
  "pet.enable": { tab: "integrations", section: "pet" },
  "pet.click_through": { tab: "integrations", section: "pet" },
  "pet.menu.corner": { tab: "integrations", section: "pet" },
  "settings.dependencies.title": { tab: "advanced", section: "dependencies" },
  "settings.browser_ext.title": { tab: "integrations", section: "extension" },
  "settings.dependencies.col_version": { tab: "advanced", section: "dependencies" },
  "settings.dependencies.col_status": { tab: "advanced", section: "dependencies" },
  "settings.advanced.section_performance": { tab: "advanced", section: "performance" },
  "settings.advanced.max_concurrent_downloads": { tab: "advanced", section: "performance" },
  "settings.advanced.max_concurrent_segments": { tab: "advanced", section: "performance" },
  "settings.advanced.concurrent_fragments": { tab: "advanced", section: "performance" },
  "settings.advanced.max_retries": { tab: "advanced", section: "performance" },
  "settings.advanced.stagger_delay": { tab: "advanced", section: "performance" },
  "settings.advanced.prevent_sleep": { tab: "advanced", section: "performance" },
  "settings.advanced.section_torrent": { tab: "advanced", section: "network" },
  "settings.advanced.torrent_listen_port": { tab: "advanced", section: "network" },
  "settings.advanced.torrent_auto_trackers": { tab: "advanced", section: "network" },
  "settings.advanced.torrent_upnp": { tab: "advanced", section: "network" },
  "settings.advanced.section_expert": { tab: "advanced", section: "expert" },
  "settings.advanced.cookies_from_browser": { tab: "advanced", section: "performance" },
  "settings.advanced.user_agent": { tab: "advanced", section: "performance" },
  "league.settings_title": { tab: "integrations", section: "league" },
  "league.settings_enable": { tab: "integrations", section: "league" },
  "league.settings_auto_accept": { tab: "integrations", section: "league" },
  "music.settings_title": { tab: "integrations", section: "music" },
  "music.settings_enable": { tab: "integrations", section: "music" },
  "video.settings_title": { tab: "integrations", section: "video" },
  "video.settings_enable": { tab: "integrations", section: "video" },
  "tube.settings_title": { tab: "integrations", section: "tube" },
  "tube.settings_enable": { tab: "integrations", section: "tube" },
  "read.settings_title": { tab: "integrations", section: "read" },
  "read.settings_enable": { tab: "integrations", section: "read" },
  "telegram.settings_title": { tab: "integrations", section: "telegram" },
  "telegram.settings_enable": { tab: "integrations", section: "telegram" },
  "washmybelly.settings_title": { tab: "integrations", section: "washmybelly" },
  "washmybelly.settings_enable": { tab: "integrations", section: "washmybelly" },
  "washmybelly.settings_quarantine_days": { tab: "integrations", section: "washmybelly" },
  "washmybelly.settings_auto_hold": { tab: "integrations", section: "washmybelly" },
  "canvas.settings.title": { tab: "integrations", section: "canvas" },
  "settings.advanced.section_debug": { tab: "advanced", section: "debug" },
  "debug.enable": { tab: "advanced", section: "debug" },
  "debug.open_panel": { tab: "advanced", section: "debug" },
  "settings.rules.title": { tab: "downloads", section: "rules" },
  "settings.rules.add": { tab: "downloads", section: "rules" },
  "settings.rules.test_title": { tab: "downloads", section: "rules" },
  "settings.storage.title": { tab: "advanced", section: "storage" },
  "settings.storage.portable_label": { tab: "advanced", section: "storage" },
  "settings.storage.dedupe_label": { tab: "advanced", section: "storage" },
  "settings.storage.silence_label": { tab: "advanced", section: "storage" },
  "settings.advanced.reset": { tab: "advanced", section: null },
};

/** Old `tab|section` pairs (section may be empty) that moved. */
const TAB_SECTION_ALIASES: Record<string, [SettingsCategory, string | null]> = {
  "advanced|league": ["integrations", "league"],
  "advanced|music": ["integrations", "music"],
  "advanced|video": ["integrations", "video"],
  "advanced|tube": ["integrations", "tube"],
  "advanced|read": ["integrations", "read"],
  "advanced|telegram": ["integrations", "telegram"],
  "advanced|washmybelly": ["integrations", "washmybelly"],
  "advanced|rules": ["downloads", "rules"],
  "advanced|torrent": ["advanced", "network"],
  "appearance|rpc": ["integrations", "rpc"],
  "appearance|workspace": ["appearance", "theme"],
  "channels|": ["downloads", "channels"],
  "cookies|": ["downloads", "cookies"],
  "dependencies|": ["advanced", "dependencies"],
  "dependencies|extension": ["integrations", "extension"],
  "downloads|hotkeys": ["downloads", "clipboard"],
  "downloads|output": ["downloads", "files"],
  "downloads|platforms": ["downloads", "sites"],
  "network|": ["advanced", "network"],
  "network|extension": ["integrations", "extension"],
  "network|proxy": ["advanced", "network"],
  "profile|": ["general", "profile"],
  "typography|": ["appearance", "typography"],
  "world|": ["integrations", "world"],
  "world|pet": ["integrations", "pet"],
};

function isCategory(value: string | null): value is SettingsCategory {
  return value !== null && (SETTINGS_CATEGORIES as readonly string[]).includes(value);
}

function isSection(tab: SettingsCategory, section: string | null): section is string {
  return section !== null && SETTINGS_SECTIONS[tab].includes(section);
}

/** Legacy `#settings-section-x`, `#settings-pet` and bare `#x` hashes name a section. */
function sectionFromHash(hash: string): string | null {
  if (!hash || hash === "#") return null;
  const raw = decodeURIComponent(hash.slice(1));
  if (raw === "settings-pet") return "pet";
  const prefixed = /^settings-section-([a-z0-9-]+)$/.exec(raw);
  if (prefixed) return prefixed[1];
  return /^[a-z][a-z0-9-]*$/.test(raw) ? raw : null;
}

export type SettingsLocation = {
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

/**
 * Resolution order (navigation-map.json): exact known setting, known tab/section alias,
 * valid new tab and section, downloads fallback. Unrelated params and the hash survive.
 */
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
    // Operational AI blocks moved out of Settings; keep the setting key and extra params.
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

/** Canonical href for a Settings destination. */
export function settingsHref(tab: SettingsCategory, section?: string | null, setting?: string | null): string {
  const params = new URLSearchParams({ tab });
  if (section) params.set("section", section);
  if (setting) params.set("setting", setting);
  return `/settings?${params}`;
}

export function parseSettingsSection(params: URLSearchParams, allowed: readonly string[]): string | null {
  const value = params.get("section");
  return value !== null && allowed.includes(value) ? value : null;
}

function oneOf<T extends string>(value: string | null, allowed: readonly T[]): T | null {
  return value !== null && (allowed as readonly string[]).includes(value) ? (value as T) : null;
}

/** `/llm/roster?bot=<id>&section=skills|learning`. The bot id is only a lookup key. */
export function parseRosterTarget(params: URLSearchParams): { bot: string | null; section: "skills" | "learning" | null } {
  const raw = params.get("bot");
  const bot = raw !== null && raw.trim() !== "" && raw.length <= 200 ? raw : null;
  return { bot, section: bot ? oneOf(params.get("section"), ["skills", "learning"] as const) : null };
}

/** `/llm/mcp?mode=server&section=permissions|media`; client is the default. */
export function parseMcpTarget(params: URLSearchParams): { mode: "client" | "server"; section: "permissions" | "media" | null } {
  const mode = oneOf(params.get("mode"), ["client", "server"] as const) ?? "client";
  return { mode, section: mode === "server" ? oneOf(params.get("section"), ["permissions", "media"] as const) : null };
}

/** `/music?view=settings&panel=eq`. */
export function parseMusicTarget(params: URLSearchParams): { view: "settings" | null; panel: "eq" | null } {
  const view = oneOf(params.get("view"), ["settings"] as const);
  return { view, panel: oneOf(params.get("panel"), ["eq"] as const) };
}

/** `/?panel=download-advanced`. */
export function parseHomeTarget(params: URLSearchParams): { panel: "download-advanced" | null } {
  return { panel: oneOf(params.get("panel"), ["download-advanced"] as const) };
}

/** `/llm/summaries?section=form|history&setting=...`; setting is preserved as given. */
export function parseSummariesTarget(params: URLSearchParams): { section: "form" | "history" | null; setting: string | null } {
  return {
    section: oneOf(params.get("section"), ["form", "history"] as const),
    setting: params.get("setting") || null,
  };
}

function prefersReducedMotion(): boolean {
  if (typeof document !== "undefined" && document.querySelector("[data-reduce-motion=\"true\"]")) return true;
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

const FOCUSABLE = "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])";

export type FocusWhenReadyOptions = {
  timeoutMs?: number;
  /** Focus the first enabled control inside the target (a `data-setting-key` row). */
  preferControl?: boolean;
  /** Called when the target never appears, or (with preferControl) has no enabled control. */
  onUnavailable?: () => void;
};

// ---- Initial navigation gate.
// On a cold load SvelteKit mounts the page, waits a few ticks and then, when the focused
// element did not change during those ticks, sends focus back to <body>
// (reset_focus <- navigate <- start). A destination focused while the page mounted is
// lost that way. The root layout holds focus requests until its first afterNavigate;
// client navigations are not gated (their focus already lands after the capture).
let initialNavigationPending = false;
let holdTimer: ReturnType<typeof setTimeout> | null = null;
const navigationWaiters = new Set<() => void>();

/** Root layout, while it initializes: focus requests wait for the first navigation. */
export function holdFocusUntilNavigated(maxMs = 3000): void {
  if (initialNavigationPending) return;
  initialNavigationPending = true;
  // Never strand a destination if afterNavigate does not come (error page, bare window).
  holdTimer = setTimeout(releaseFocusAfterNavigation, maxMs);
}

/** Root layout's first afterNavigate: runs the held focus requests. */
export function releaseFocusAfterNavigation(): void {
  if (!initialNavigationPending) return;
  initialNavigationPending = false;
  if (holdTimer) clearTimeout(holdTimer);
  holdTimer = null;
  const waiters = [...navigationWaiters];
  navigationWaiters.clear();
  for (const run of waiters) run();
}

// ---- Keep a destination in view while the content above it settles.
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

/** True when the target's top edge is outside the visible part of its scroller. */
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

/**
 * Late content above a destination (the limits strip, a lazy panel) can push it off
 * screen right after the first scroll. For a short window, re-scroll it into view when
 * a surrounding box resizes. The user wins: any scroll, click or key, or focus moving
 * elsewhere, stops it.
 */
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

/**
 * Waits for `selector` (late-loading destinations), reveals collapsed disclosures around it,
 * scrolls it into view and focuses it. Sections get tabindex=-1. On a cold load it starts
 * after the initial navigation (see holdFocusUntilNavigated), and for a short while it
 * keeps the target in view as the layout settles. Returns a cleanup.
 */
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
    // A collapsed disclosure opens asynchronously; the observer retries once it is visible.
    if (revealAncestors(target)) return false;
    let focusTarget: HTMLElement;
    if (opts.preferControl) {
      const control = target.matches(FOCUSABLE) ? target : target.querySelector<HTMLElement>(FOCUSABLE);
      if (!control) {
        // Conditional or disabled control: never focus an unrelated one.
        finish();
        opts.onUnavailable?.();
        return true;
      }
      focusTarget = control;
    } else {
      // A section link to a collapsed group opens it: the user asked for that section.
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

/** Asks collapsed disclosure regions (SettingGroup advanced) around `node` to open. */
export function revealAncestors(node: HTMLElement): boolean {
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
