import i18n from "sveltekit-i18n";
import type { Readable } from "svelte/store";
import en from "./en.json";

type Payload = [payload?: Record<string, unknown>];

const config = {
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
    {
      locale: "lo",
      key: "",
      loader: async () => (await import("./lo.json")).default,
    },
  ],
};

export const defaultLocale = "en";

const RTL_LOCALES = ["fa", "ar", "he"];

export function isRtlLocale(l: string | null | undefined): boolean {
  return !!l && RTL_LOCALES.includes(l);
}

const i18nInstance = new i18n<Payload>(config);

export const { t, locale, locales, loading, loadTranslations } = i18nInstance;

export const rawTranslations = i18nInstance.translations as unknown as Readable<
  Record<string, Record<string, unknown>>
>;
