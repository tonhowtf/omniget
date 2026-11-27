import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { updateSettings, getSettings, type AppSettings } from "$lib/stores/settings-store.svelte";
import { t } from "$lib/i18n";
import { get } from "svelte/store";
import { showToast } from "$lib/stores/toast-store.svelte";
import { parseSettingNumber } from "$lib/settings-search";
import { failureShownInline } from "$lib/stores/settings-status-surfaces";

export async function saveSettings(partial: Record<string, unknown>): Promise<AppSettings | null> {
  try { return await updateSettings(partial); }
  catch {
    if (!failureShownInline(partial)) showToast("error", get(t)("settings.save_failed"));
    return null;
  }
}

export async function chooseFolder() {
  const tt = get(t);
  const selected = await open({
    directory: true,
    title: tt("settings.download.default_output_dir") as string,
  });
  if (selected) {
    await saveSettings({ download: { default_output_dir: selected } });
  }
}

export async function changeLanguage(e: Event) {
  const value = (e.target as HTMLSelectElement).value;
  await saveSettings({ appearance: { language: value } });
}

export async function toggleBool(section: string, key: string, current: boolean) {
  await saveSettings({ [section]: { [key]: !current } });
}

export async function changeNumber(section: string, key: string, e: Event) {
  const input = e.target as HTMLInputElement;
  const value = parseSettingNumber(input.value, input.min, input.max);
  if (value === null || !input.validity.valid) {
    input.reportValidity();
    return;
  }
  if (!await saveSettings({ [section]: { [key]: value } })) return;
  if (key === "max_concurrent_downloads") {
    try { await invoke("update_max_concurrent", { max: value }); }
    catch { showToast("error", get(t)("settings.save_failed")); }
  }
}

export type DependencyStatus = {
  name: string;
  installed: boolean;
  version: string | null;
  source: string;
  path: string | null;
  outdated: boolean;
};

export const MORE_THEMES = [
  { id: "catppuccin-latte", label: "Catppuccin Latte" },
  { id: "catppuccin-frappe", label: "Catppuccin Frappé" },
  { id: "catppuccin-macchiato", label: "Catppuccin Macchiato" },
  { id: "catppuccin-mocha", label: "Catppuccin Mocha" },
  { id: "one-dark-pro", label: "One Dark Pro" },
  { id: "dracula", label: "Dracula" },
  { id: "nyxvamp-veil", label: "NyxVamp Veil" },
  { id: "nyxvamp-obsidian", label: "NyxVamp Obsidian" },
  { id: "nyxvamp-radiance", label: "NyxVamp Radiance" },
  { id: "eink-day", label: "E-ink Day" },
  { id: "eink-sepia", label: "E-ink Sepia" },
  { id: "eink-night", label: "E-ink Night" },
];

export { getSettings };
