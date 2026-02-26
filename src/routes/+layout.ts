import { loadTranslations, defaultLocale } from "$lib/i18n";
import { loadSettings } from "$lib/stores/settings-store.svelte";

export const ssr = false;

export const load = async ({ url }) => {
  let lang = defaultLocale;
  try {
    const settings = await loadSettings();
    lang = settings.appearance?.language || defaultLocale;
  } catch {
    // First run or settings unavailable — use defaultLocale
  }
  await loadTranslations(lang, url.pathname);
  return {};
};
