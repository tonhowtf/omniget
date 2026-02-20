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
    filename_template: string;
    organize_by_platform: boolean;
    download_subtitles: boolean;
  };
  advanced: {
    max_concurrent_segments: number;
    max_retries: number;
    max_concurrent_downloads: number;
    concurrent_fragments: number;
    stagger_delay_ms: number;
  };
  telegram: {
    concurrent_downloads: number;
    fix_file_extensions: boolean;
  };
};

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
  return result;
}

export async function updateSettings(partial: Record<string, unknown>): Promise<AppSettings> {
  const result = await invoke<AppSettings>("update_settings", {
    partial: JSON.stringify(partial),
  });
  settings = result;
  applyTheme(result.appearance.theme);
  return result;
}

export async function resetSettings(): Promise<AppSettings> {
  const result = await invoke<AppSettings>("reset_settings");
  settings = result;
  applyTheme(result.appearance.theme);
  return result;
}
