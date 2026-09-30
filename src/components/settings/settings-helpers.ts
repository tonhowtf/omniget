import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { updateSettings, getSettings, type AppSettings } from "$lib/stores/settings-store.svelte";
import { t } from "$lib/i18n";
import { get } from "svelte/store";
import { showToast } from "$lib/stores/toast-store.svelte";
import { parseSettingNumber } from "$lib/settings-search";
import { failureShownInline } from "$lib/stores/settings-status-surfaces";

/**
 * Autosave from an event handler. A failure is announced once: by the field's inline
 * SettingStatus when every changed field has one mounted, otherwise by a toast.
 */
export async function saveSettings(partial: Record<string, unknown>): Promise<AppSettings | null> {
  try { return await updateSettings(partial); }
  catch {
    if (!failureShownInline(partial)) showToast("error", get(t)("settings.save_failed"));
    return null;
  }
}


export async function chooseCookieFile() {
  const selected = await open({
    title: get(t)("settings.cookies.import_file_dialog_title"),
    filters: [{ name: "Cookies", extensions: ["txt"] }],
    multiple: false,
  });
  if (selected && typeof selected === "string") {
    await saveSettings({ download: { cookie_file: selected } });
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

/** Core modes (system/light/dark) and named themes both go through mode/preset. */
export async function changeTheme(value: string) {
  await saveSettings({ appearance: { theme: value } });
}

/** The store applies the confirmed language through the layout's locale applier. */
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

export async function changeQuality(e: Event) {
  const value = (e.target as HTMLSelectElement).value;
  await saveSettings({ download: { video_quality: value } });
}

export type DependencyStatus = {
  name: string;
  installed: boolean;
  version: string | null;
  /** "managed" | "system" | "flatpak" | "missing" */
  source: string;
  path: string | null;
  outdated: boolean;
};

export async function loadDeps(): Promise<DependencyStatus[]> {
  try {
    return await invoke<DependencyStatus[]>("check_dependencies");
  } catch {
    return [];
  }
}

export const CORE_THEMES = [
  { id: "system", labelKey: "settings.appearance.theme_system", colors: null as string[] | null },
  { id: "light", labelKey: "design.light", label: "Light", colors: ["#F6F4F0", "#1E1C19", "#FF9D24"] },
  { id: "dark", labelKey: "design.dark", label: "Dark", colors: ["#1B1B1D", "#F4F2EE", "#FF9D24"] },
];

export const MORE_THEMES = [
  { id: "catppuccin-latte", label: "Catppuccin Latte", colors: ["#eff1f5", "#4c4f69", "#1e66f5"] },
  { id: "catppuccin-frappe", label: "Catppuccin Frappé", colors: ["#303446", "#c6d0f5", "#8caaee"] },
  { id: "catppuccin-macchiato", label: "Catppuccin Macchiato", colors: ["#24273a", "#cad3f5", "#8aadf4"] },
  { id: "catppuccin-mocha", label: "Catppuccin Mocha", colors: ["#1e1e2e", "#cdd6f4", "#89b4fa"] },
  { id: "one-dark-pro", label: "One Dark Pro", colors: ["#282c34", "#abb2bf", "#61afef"] },
  { id: "dracula", label: "Dracula", colors: ["#22212C", "#F8F8F2", "#9580FF"] },
  { id: "nyxvamp-veil", label: "NyxVamp Veil", colors: ["#1E1E2E", "#D9E0EE", "#F28FAD"] },
  { id: "nyxvamp-obsidian", label: "NyxVamp Obsidian", colors: ["#000A0F", "#C0C0CE", "#F28FAD"] },
  { id: "nyxvamp-radiance", label: "NyxVamp Radiance", colors: ["#F7F7FF", "#1E1E2E", "#9655FF"] },
  { id: "eink-day", label: "E-ink Day", colors: ["#f5f2ea", "#1d1d1b", "#2b2b28"] },
  { id: "eink-sepia", label: "E-ink Sepia", colors: ["#f0e6d2", "#3f2e20", "#7a4a22"] },
  { id: "eink-night", label: "E-ink Night", colors: ["#0a0a0a", "#d4d4d0", "#b0b0ab"] },
];

export const MORE_THEME_IDS = new Set(MORE_THEMES.map((t) => t.id));

export { getSettings, updateSettings };
