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
  };
  advanced: {
    max_concurrent_segments: number;
    max_retries: number;
    debug_mode: boolean;
  };
};

let settings = $state<AppSettings | null>(null);

export function getSettings(): AppSettings | null {
  return settings;
}

export async function loadSettings(): Promise<AppSettings> {
  const result = await invoke<AppSettings>("get_settings");
  settings = result;
  return result;
}

export async function updateSettings(partial: Record<string, unknown>): Promise<AppSettings> {
  const result = await invoke<AppSettings>("update_settings", {
    partial: JSON.stringify(partial),
  });
  settings = result;
  return result;
}

export async function resetSettings(): Promise<AppSettings> {
  const result = await invoke<AppSettings>("reset_settings");
  settings = result;
  return result;
}
