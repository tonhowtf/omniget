import { loadTranslations, defaultLocale } from "$lib/i18n";

export const ssr = false;

export const load = async ({ url }) => {
  await loadTranslations(defaultLocale, url.pathname);
  return {};
};
