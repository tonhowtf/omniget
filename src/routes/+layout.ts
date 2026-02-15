import { loadTranslations, defaultLocale } from "$lib/i18n";

export const ssr = false;

export const load = async ({ url }) => {
  const browserLang = navigator.language || defaultLocale;
  const locale = browserLang.startsWith("en") ? "en" : "pt";
  await loadTranslations(locale, url.pathname);
  return {};
};
