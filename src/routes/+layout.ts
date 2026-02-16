import { loadTranslations, defaultLocale } from "$lib/i18n";
import { invoke } from "@tauri-apps/api/core";

export const ssr = false;

export const load = async ({ url }) => {
  let lang = defaultLocale;
  try {
    const settings = await invoke<{ appearance: { language: string } }>("get_settings");
    lang = settings.appearance?.language || defaultLocale;
  } catch {
    // First run or settings unavailable — use defaultLocale
  }
  await loadTranslations(lang, url.pathname);
  return {};
};
