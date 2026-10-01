// Live language: the Rust side emits `app://language` when the app language
// changes (the settings screen, or `app_settings_set appearance.language`
// over MCP). Every window that imports this switches its translations right
// away instead of waiting for a reload. Importing it twice is harmless.

import { listen } from '@tauri-apps/api/event';
import { loadTranslations, locale } from '$lib/i18n';

const EVENT_LANGUAGE = 'app://language';

let started = false;

async function applyLanguage(language: string): Promise<void> {
  const lang = language.trim();
  if (!lang) return;
  try {
    await loadTranslations(lang, typeof location === 'undefined' ? '/' : location.pathname);
  } catch (e) {
    console.warn(`[i18n] could not load "${lang}":`, e);
  }
  locale.set(lang);
}

function followAppLanguage(): void {
  if (started || typeof window === 'undefined') return;
  started = true;
  void listen<{ language?: string }>(EVENT_LANGUAGE, (e) => {
    if (typeof e.payload?.language === 'string') void applyLanguage(e.payload.language);
  }).catch(() => {
    started = false;
  });
}

followAppLanguage();
