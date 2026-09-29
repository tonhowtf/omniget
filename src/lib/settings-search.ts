export type SettingsCategory = "downloads" | "appearance" | "profile" | "typography" | "network" | "cookies" | "channels" | "ai" | "world" | "dependencies" | "advanced";

export type SettingSearchEntry = {
  key: string;
  category: SettingsCategory;
  section: string;
  sectionKey: string;
};

// Search metadata exists independently of mounted panels, enabled features and visual CSS.
const groups: [SettingsCategory, string, string, string[]][] = [
  ["appearance", "workspace", "design.title", ["design.title", "design.hint", "design.system", "design.light", "design.dark"]],
  ["appearance", "", "settings.cat_appearance", ["settings.cat_appearance", "settings.general.start_with_system", "settings.general.start_minimized", "settings.general.exit_on_close", "settings.general.run_in_background", "settings.appearance.reduce_motion", "settings.appearance.reduce_transparency", "settings.appearance.theme", "settings.appearance.language", "settings.general.reset_hints"]],
  ["appearance", "rpc", "settings.rpc.title", ["settings.rpc.title", "settings.rpc.enable", "settings.rpc.test", "settings.rpc.app_id", "settings.rpc.large_image"]],
  ["profile", "", "profile.title", ["profile.title", "profile.desc"]],
  ["typography", "", "settings.typography.section_title", ["settings.typography.section_title", "settings.typography.presets_title", "settings.typography.fonts_title", "settings.typography.line_height_label", "settings.typography.spacing_label", "settings.typography.preview_title", "settings.typography.display_label", "settings.typography.body_label", "settings.typography.mono_label"]],
  ["downloads", "output", "settings.download.section_output", ["settings.download.section_output", "settings.download.default_output_dir", "settings.download.organize_by_platform", "settings.download.skip_existing", "settings.download.filename_template"]],
  ["downloads", "quality", "settings.download.section_quality", ["settings.download.section_quality", "settings.download.video_quality", "settings.download.speed_limit", "settings.download.always_ask_path"]],
  ["downloads", "subtitles", "settings.download.section_subtitles", ["settings.download.section_subtitles", "settings.download.download_subtitles", "settings.download.include_auto_subtitles", "settings.download.caption_locale", "settings.download.keep_vtt", "settings.download.translate_metadata", "settings.download.embed_metadata", "settings.download.embed_thumbnail", "settings.download.write_nfo_sidecar"]],
  ["downloads", "hotkeys", "settings.download.section_clipboard_hotkeys", ["settings.download.section_clipboard_hotkeys", "settings.download.clipboard_detection", "settings.download.auto_download_on_paste", "settings.download.hotkey_enabled", "settings.download.hotkey_binding", "settings.download.copy_to_clipboard_on_hotkey", "settings.download.music_hotkey_enabled", "settings.download.music_hotkey_binding", "settings.download.music_audio_format"]],
  ["downloads", "platforms", "settings.download.section_per_platform", ["settings.download.section_per_platform", "settings.download.youtube_sponsorblock", "settings.download.sb_mode", "settings.download.sb_categories", "settings.download.split_by_chapters", "settings.download.live_from_start", "settings.download.bilibili_container_label", "settings.download.bilibili_danmaku_label", "settings.download.bilibili_danmaku_format_label", "settings.download.bilibili_nfo_label", "settings.download.bilibili_cover_sidecar_label", "settings.download.bilibili_cover_format_label", "settings.download.bilibili_naming_label", "settings.download.bilibili_cdn_label", "settings.download.bilibili_cdn_prefer_label"]],
  ["network", "extension", "settings.bridge.title", ["settings.bridge.title", "settings.bridge.pair_label", "settings.bridge.endpoint_label", "settings.bridge.token_label", "settings.bridge.rotate_label"]],
  ["network", "proxy", "settings.proxy.title", ["settings.proxy.title", "settings.proxy.enabled", "settings.proxy.type", "settings.proxy.host", "settings.proxy.port", "settings.proxy.username", "settings.proxy.password"]],
  ["cookies", "", "settings.cookies.title", ["settings.cookies.title", "settings.cookies.managed_only_title", "settings.cookies.capture_via_extension", "settings.cookies.import_trigger", "settings.cookies.action_test"]],
  ["channels", "", "settings.channels.title", ["settings.channels.title", "settings.channels.add", "settings.channels.auto_download", "settings.channels.interval", "settings.channels.check_now"]],
  ["ai", "", "settings.ai.title", ["settings.ai.title", "settings.ai.provider", "settings.ai.model", "settings.ai.openai_key", "settings.ai.anthropic_key", "settings.ai.local_url", "settings.ai.test", "settings.ai.summarize_title", "settings.ai.history"]],
  ["world", "", "world.settings.title", ["world.settings.title", "world.settings.enabled", "world.settings.thinking", "world.settings.room_server", "world.settings.tier_measured", "world.settings.tier_pinned", "pet.enable", "pet.click_through", "pet.menu.corner"]],
  ["dependencies", "", "settings.dependencies.title", ["settings.dependencies.title", "settings.browser_ext.title", "settings.dependencies.col_version", "settings.dependencies.col_status"]],
  ["advanced", "performance", "settings.advanced.section_performance", ["settings.advanced.section_performance", "settings.advanced.max_concurrent_downloads", "settings.advanced.max_concurrent_segments", "settings.advanced.concurrent_fragments", "settings.advanced.max_retries", "settings.advanced.stagger_delay", "settings.advanced.prevent_sleep"]],
  ["advanced", "torrent", "settings.advanced.section_torrent", ["settings.advanced.section_torrent", "settings.advanced.torrent_listen_port", "settings.advanced.torrent_auto_trackers", "settings.advanced.torrent_upnp"]],
  ["advanced", "expert", "settings.advanced.section_expert", ["settings.advanced.section_expert", "settings.advanced.cookies_from_browser", "settings.advanced.user_agent"]],
  ["advanced", "league", "league.settings_title", ["league.settings_title", "league.settings_enable", "league.settings_auto_accept"]],
  ["advanced", "music", "music.settings_title", ["music.settings_title", "music.settings_enable"]],
  ["advanced", "video", "video.settings_title", ["video.settings_title", "video.settings_enable"]],
  ["advanced", "washmybelly", "washmybelly.settings_title", ["washmybelly.settings_title", "washmybelly.settings_enable", "washmybelly.settings_quarantine_days", "washmybelly.settings_auto_hold"]],
  ["advanced", "debug", "settings.advanced.section_debug", ["settings.advanced.section_debug", "debug.enable", "debug.open_panel"]],
  ["advanced", "rules", "settings.rules.title", ["settings.rules.title", "settings.rules.add", "settings.rules.test_title"]],
  ["advanced", "storage", "settings.storage.title", ["settings.storage.title", "settings.storage.portable_label", "settings.storage.dedupe_label", "settings.storage.silence_label"]],
  ["advanced", "", "settings.advanced.reset", ["settings.advanced.reset"]],
];

export const SETTINGS_SEARCH_INDEX: SettingSearchEntry[] = groups.flatMap(([category, section, sectionKey, keys]) =>
  keys.map(key => ({ key, category, section, sectionKey })),
);

export function normalizeSettingsQuery(text: string): string {
  return text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLocaleLowerCase().trim();
}

export function searchSettings(query: string, translate: (key: string) => string): SettingSearchEntry[] {
  const terms = normalizeSettingsQuery(query).split(/\s+/).filter(Boolean);
  if (!terms.length) return [];
  return SETTINGS_SEARCH_INDEX.filter(entry => {
    const keys = [entry.key, entry.sectionKey, `${entry.key}_desc`, `${entry.key}_hint`];
    const text = normalizeSettingsQuery(keys.map(key => {
      const value = translate(key);
      return value === key ? "" : value;
    }).join(" "));
    return terms.every(term => text.includes(term));
  });
}

export function settingsEntryUrl(entry: SettingSearchEntry): string {
  const params = new URLSearchParams({ tab: entry.category, setting: entry.key });
  if (entry.section) params.set("section", entry.section);
  return `/settings?${params}`;
}

/** Native numeric inputs expose their declared limits; zero is valid when min permits it. */
export function parseSettingNumber(value: string, min: string, max: string): number | null {
  if (!value.trim()) return null;
  const number = Number(value);
  if (!Number.isFinite(number) || !Number.isInteger(number)) return null;
  if (min !== "" && number < Number(min)) return null;
  if (max !== "" && number > Number(max)) return null;
  return number;
}
