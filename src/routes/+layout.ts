import { loadTranslations } from "$lib/i18n";
import {
  applyFallbackPresentation,
  fallbackLocale,
  getSettings,
  loadSettings,
} from "$lib/stores/settings-store.svelte";

export const ssr = false;

// The shell never waits on the settings read: a slow or hung `settings_get_v3` used to
// leave the whole window blank. A fast answer still picks the first language, so a
// normal start does not flash the fallback; otherwise the OS language and light/dark
// hold until the store publishes (the layout's locale applier then switches).
const FIRST_READ_GRACE_MS = 250;

let firstRead: Promise<unknown> | null = null;

export const load = async ({ url }) => {
  let lang = getSettings()?.appearance?.language;
  if (!lang) {
    applyFallbackPresentation();
    if (!firstRead) {
      firstRead = loadSettings().catch(() => {
        // Unavailable or failed: Settings shows the error state with a retry.
      });
    }
    await Promise.race([firstRead, new Promise((resolve) => setTimeout(resolve, FIRST_READ_GRACE_MS))]);
    lang = getSettings()?.appearance?.language || fallbackLocale();
  }
  try {
    await loadTranslations(lang, url.pathname);
  } catch (e) {
    // O inglês estático em `$lib/i18n` segura a UI; só registra o motivo.
    console.warn(`[i18n] could not load "${lang}":`, e);
  }
  return {};
};
