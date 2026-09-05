import i18n from "sveltekit-i18n";
import en from "./en.json";

type Payload = [payload?: Record<string, unknown>];

const config = {
  // O inglês vai embutido no bundle principal e serve de fallback: se o
  // carregamento assíncrono do idioma falhar no webview, a UI nunca fica sem
  // texto (era o sintoma no app empacotado: todos os `$t` vazios).
  initLocale: "en",
  fallbackLocale: "en",
  translations: { en },
  loaders: [
    
    {
      locale: "en",
      key: "",
      loader: async () => (await import("./en.json")).default,
    },
	{
      locale: "ru",
      key: "",
      loader: async () => (await import("./ru.json")).default,
    },
    {
      locale: "el",
      key: "",
      loader: async () => (await import("./el.json")).default,
    },
    {
      locale: "pt",
      key: "",
      loader: async () => (await import("./pt.json")).default,
    },
    {
      locale: "zh",
      key: "",
      loader: async () => (await import("./zh.json")).default,
    },
    {
      locale: "zh-TW",
      key: "",
      loader: async () => (await import("./zh-TW.json")).default,
    },
    {
      locale: "ja",
      key: "",
      loader: async () => (await import("./ja.json")).default,
    },
    {
      locale: "it",
      key: "",
      loader: async () => (await import("./it.json")).default,
    },
    {
      locale: "fr",
      key: "",
      loader: async () => (await import("./fr.json")).default,
    },
    {
      locale: "es",
      key: "",
      loader: async () => (await import("./es.json")).default,
    },
    {
      locale: "fa",
      key: "",
      loader: async () => (await import("./fa.json")).default,
    },
  ],
};

export const defaultLocale = "en";

export const RTL_LOCALES = ["fa", "ar", "he"];

export function isRtlLocale(l: string | null | undefined): boolean {
  return !!l && RTL_LOCALES.includes(l);
}

export const { t, locale, locales, loading, loadTranslations } = new i18n<Payload>(config);
