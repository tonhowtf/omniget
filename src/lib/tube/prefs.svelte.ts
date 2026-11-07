import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
import type { TubeSettings } from "./types";

const TUBE_DEFAULTS: TubeSettings = {
  enabled: true,
  look: "classic",
  start_face: "video",
  start_page: "home",
  use_account: true,
  default_quality: "auto",
  prefer_codec: "auto",
  audio_quality: "high",
  autoplay_next: true,
  autoplay_on_open: true,
  preview_on_hover: true,
  normalize_volume: true,
  send_history: true,
  endless_radio: true,
  sponsorblock: true,
  sb_categories: {
    sponsor: "skip",
    selfpromo: "ask",
    interaction: "ask",
    intro: "show",
    outro: "show",
    preview: "show",
    hook: "show",
    music_offtopic: "skip",
    filler: "off",
    poi_highlight: "show",
    exclusive_access: "show",
  },
  sb_server: "https://sponsor.ajay.app",
  sb_whitelist: [],
  sb_download: "off",
  dearrow: false,
  dearrow_thumbs: false,
  ryd: true,
  hide_shorts: false,
  hide_live: false,
  hide_upcoming: false,
  hide_members: true,
  hide_mixes: true,
  hide_playables: true,
  hide_posts: false,
  hide_watched: false,
  watched_threshold: 85,
  hide_home_feed: false,
  hide_related: false,
  hide_comments: false,
  hide_chips: false,
  hide_metrics: false,
  infinite_scroll: true,
  grid_columns: 0,
  blocked_channels: [],
  blocked_words: [],
  whole_word: true,
  min_duration_s: 0,
  max_duration_s: 0,
  lyrics_provider: "auto",
  download_folder: null,
  storage_cap_gb: 0,
};

export function tubePrefs(): TubeSettings {
  const s = getSettings()?.tube ?? {};
  return { ...TUBE_DEFAULTS, ...s, sb_categories: { ...TUBE_DEFAULTS.sb_categories, ...(s.sb_categories ?? {}) } };
}

export async function patchTubePrefs(patch: Partial<TubeSettings>): Promise<void> {
  const cur = tubePrefs();
  await updateSettings({ tube: { ...cur, ...patch } });
}
