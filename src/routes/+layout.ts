import { loadTranslations } from "$lib/i18n";
import {
  applyFallbackPresentation,
  fallbackLocale,
  getSettings,
  loadSettings,
} from "$lib/stores/settings-store.svelte";

export const ssr = false;

const FIRST_READ_GRACE_MS = 250;

let firstRead: Promise<unknown> | null = null;

export const load = async ({ url }) => {
  let lang = getSettings()?.appearance?.language;
  if (!lang) {
    applyFallbackPresentation();
    if (!firstRead) {
      firstRead = loadSettings().catch(() => {
      });
    }
    await Promise.race([firstRead, new Promise((resolve) => setTimeout(resolve, FIRST_READ_GRACE_MS))]);
    lang = getSettings()?.appearance?.language || fallbackLocale();
  }
  try {
    await loadTranslations(lang, url.pathname);
  } catch (e) {
    console.warn(`[i18n] could not load "${lang}":`, e);
  }
  return {};
};
