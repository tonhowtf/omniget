import { invoke } from "@tauri-apps/api/core";

export type AppSettings = {
  schema_version: number;
  appearance: {
    theme: string;
    language: string;
  };
  download: {
    default_output_dir: string;
    always_ask_path: boolean;
    video_quality: string;
    skip_existing: boolean;
    download_attachments: boolean;
    download_descriptions: boolean;
    embed_metadata: boolean;
    embed_thumbnail: boolean;
    clipboard_detection: boolean;
    auto_download_on_paste: boolean;
    filename_template: string;
    organize_by_platform: boolean;
    download_subtitles: boolean;
    include_auto_subtitles: boolean;
    caption_locale: string;
    keep_vtt: boolean;
    continuous_lecture_numbers: boolean;
    translate_metadata: boolean;
    youtube_sponsorblock: boolean;
    sponsorblock_mode: string;
    sponsorblock_categories: string[];
    split_by_chapters: boolean;
    live_from_start: boolean;
    speed_limit: string;
    hotkey_enabled: boolean;
    hotkey_binding: string;
    clip_hotkey_enabled?: boolean;
    clip_hotkey_binding?: string;
    music_hotkey_enabled: boolean;
    music_hotkey_binding: string;
    music_audio_format: string;
    copy_to_clipboard_on_hotkey: boolean;
    extra_ytdlp_flags?: string[];
    cookie_file: string;
    always_use_managed_cookies: boolean;
    bilibili_danmaku_enabled: boolean;
    bilibili_danmaku_format: string;
    bilibili_container: string;
    bilibili_nfo_enabled: boolean;
    bilibili_cover_sidecar: boolean;
    bilibili_cover_format: string;
    bilibili_naming_video: string;
    bilibili_naming_multi_part: string;
    bilibili_naming_bangumi: string;
    bilibili_naming_cheese: string;
    bilibili_naming_collection: string;
    bilibili_cdn_hosts: string;
    bilibili_cdn_prefer_alternatives: boolean;
    bilibili_preferred_qn: number;
    bilibili_preferred_codec: number;
    bilibili_preferred_audio_qn: number;
  };
  proxy?: {
    enabled?: boolean;
    proxy_type?: string;
    host?: string;
    port?: number;
    username?: string;
    password?: string;
  };
  advanced: {
    max_concurrent_segments: number;
    max_retries: number;
    max_concurrent_downloads: number;
    concurrent_fragments: number;
    stagger_delay_ms: number;
    torrent_listen_port: number;
    torrent_auto_trackers: boolean;
    torrent_upnp: boolean;
    prevent_sleep: boolean;
    cookies_from_browser: string;
    twitter_manual_cookie: string;
    user_agent: string;
  };
  telegram: {
    concurrent_downloads: number;
    fix_file_extensions: boolean;
  };
  rpc: {
    enabled: boolean;
    app_id: string;
    large_image_key: string;
  };
  onboarding_completed: boolean;
  start_with_system: boolean;
  start_minimized: boolean;
  legal_acknowledged?: boolean;
  last_download_options?: {
    mode?: "auto" | "audio" | "mute" | null;
    quality?: string | null;
  };
  typography?: TypographySettings;
};

export type TypographySettings = {
  font_display: string;
  font_body: string;
  font_mono: string;
  line_height_base: number;
  spacing_scale: number;
  preset_name: string | null;
};

const TYPOGRAPHY_DEFAULTS: TypographySettings = {
  font_display: "Bricolage Grotesque Variable",
  font_body: "Inter",
  font_mono: "IBM Plex Mono",
  line_height_base: 1.55,
  spacing_scale: 1.0,
  preset_name: "omniget-default",
};

const SPACING_BASE_PX: Record<string, number> = {
  "--space-1": 4,
  "--space-2": 8,
  "--space-3": 12,
  "--space-4": 16,
  "--space-5": 24,
  "--space-6": 32,
  "--space-7": 48,
  "--space-8": 64,
  "--space-9": 96,
};

function fontStack(fontDisplay: string): string {
  const display = fontDisplay.trim();
  if (display === "Bricolage Grotesque Variable") {
    return `'Bricolage Grotesque Variable', 'Bricolage Grotesque', ui-sans-serif, system-ui, sans-serif`;
  }
  if (display === "Inter") {
    return `'Inter', 'DM Sans', ui-sans-serif, system-ui, sans-serif`;
  }
  if (display === "DM Sans") {
    return `'DM Sans', 'Inter', ui-sans-serif, system-ui, sans-serif`;
  }
  if (display === "IBM Plex Mono") {
    return `'IBM Plex Mono', ui-monospace, monospace`;
  }
  if (display === "system-ui") {
    return `system-ui, -apple-system, BlinkMacSystemFont, sans-serif`;
  }
  return `'${display}', ui-sans-serif, system-ui, sans-serif`;
}

function bodyFontStack(fontBody: string): string {
  const body = fontBody.trim();
  if (body === "Inter") {
    return `'Inter', 'DM Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif`;
  }
  if (body === "DM Sans") {
    return `'DM Sans', 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif`;
  }
  if (body === "Bricolage Grotesque Variable") {
    return `'Bricolage Grotesque Variable', 'Bricolage Grotesque', ui-sans-serif, system-ui, sans-serif`;
  }
  if (body === "IBM Plex Mono") {
    return `'IBM Plex Mono', ui-monospace, monospace`;
  }
  if (body === "system-ui") {
    return `system-ui, -apple-system, BlinkMacSystemFont, sans-serif`;
  }
  return `'${body}', ui-sans-serif, system-ui, sans-serif`;
}

function monoFontStack(fontMono: string): string {
  const mono = fontMono.trim();
  if (mono === "IBM Plex Mono") {
    return `'IBM Plex Mono', 'DejaVu Sans Mono', 'Liberation Mono', 'Courier New', monospace`;
  }
  return `'${mono}', ui-monospace, 'Courier New', monospace`;
}

function applyTypography(typo: TypographySettings | undefined) {
  if (typeof document === "undefined") return;
  const t = typo ?? TYPOGRAPHY_DEFAULTS;
  const root = document.documentElement.style;
  root.setProperty("--font-display", fontStack(t.font_display));
  root.setProperty("--font-body", bodyFontStack(t.font_body));
  root.setProperty("--font-mono", monoFontStack(t.font_mono));
  root.setProperty("--leading-base", String(t.line_height_base));
  const scale = typeof t.spacing_scale === "number" ? t.spacing_scale : 1.0;
  if (Math.abs(scale - 1.0) > 0.001) {
    for (const [key, base] of Object.entries(SPACING_BASE_PX)) {
      root.setProperty(key, `${(base * scale).toFixed(2)}px`);
    }
  } else {
    for (const key of Object.keys(SPACING_BASE_PX)) {
      root.removeProperty(key);
    }
  }
}

let settings = $state<AppSettings | null>(null);

function applyTheme(theme: string) {
  if (typeof document === "undefined") return;

  if (theme === "system") {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.setAttribute("data-theme", prefersDark ? "dark" : "light");
  } else {
    document.documentElement.setAttribute("data-theme", theme);
  }
}

export function getSettings(): AppSettings | null {
  return settings;
}

export async function loadSettings(): Promise<AppSettings> {
  const result = await invoke<AppSettings>("get_settings");
  settings = result;
  applyTheme(result.appearance.theme);
  applyTypography(result.typography);
  return result;
}

export async function updateSettings(partial: Record<string, unknown>): Promise<AppSettings> {
  const result = await invoke<AppSettings>("update_settings", {
    partial: JSON.stringify(partial),
  });
  settings = result;
  applyTheme(result.appearance.theme);
  applyTypography(result.typography);
  return result;
}

export async function resetSettings(): Promise<AppSettings> {
  const result = await invoke<AppSettings>("reset_settings");
  settings = result;
  applyTheme(result.appearance.theme);
  applyTypography(result.typography);
  return result;
}
