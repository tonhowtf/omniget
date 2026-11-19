import { I18n } from "sveltekit-i18n";
import stores from "@sveltekit-i18n/extension-stores";
import { derived, type Readable, type Writable } from "svelte/store";
import en from "./en.json";

const config = {
  initLocale: "en",
  fallbackLocale: "en",
  translations: { en },
  loaders: [
    
    {
      locale: "en",
      namespace: "",
      loader: async () => (await import("./en.json")).default,
    },
	{
      locale: "ru",
      namespace: "",
      loader: async () => (await import("./ru.json")).default,
    },
    {
      locale: "el",
      namespace: "",
      loader: async () => (await import("./el.json")).default,
    },
    {
      locale: "pt",
      namespace: "",
      loader: async () => (await import("./pt.json")).default,
    },
    {
      locale: "zh",
      namespace: "",
      loader: async () => (await import("./zh.json")).default,
    },
    {
      locale: "zh-TW",
      namespace: "",
      loader: async () => (await import("./zh-TW.json")).default,
    },
    {
      locale: "ja",
      namespace: "",
      loader: async () => (await import("./ja.json")).default,
    },
    {
      locale: "it",
      namespace: "",
      loader: async () => (await import("./it.json")).default,
    },
    {
      locale: "fr",
      namespace: "",
      loader: async () => (await import("./fr.json")).default,
    },
    {
      locale: "es",
      namespace: "",
      loader: async () => (await import("./es.json")).default,
    },
    {
      locale: "fa",
      namespace: "",
      loader: async () => (await import("./fa.json")).default,
    },
    {
      locale: "lo",
      namespace: "",
      loader: async () => (await import("./lo.json")).default,
    },
  ],
};

const RTL_LOCALES = ["fa", "ar", "he"];

export function isRtlLocale(l: string | null | undefined): boolean {
  return !!l && RTL_LOCALES.includes(l);
}

const i18nInstance = stores(new I18n(config));

export const { locales, loading, loadTranslations } = i18nInstance;

export const t = derived(i18nInstance.t, ($t) => (key: string, vars?: Record<string, unknown>): string => {
  const text = $t(key, vars);
  if (text !== key || !vars?.default) return text;
  return String(vars.default).replace(/\{\{\s*(\w+)\s*\}\}/g, (_, name: string) => String(vars[name] ?? ""));
});

export const locale = i18nInstance.locale as unknown as Writable<string>;

export const rawTranslations = i18nInstance.translations as unknown as Readable<
  Record<string, Record<string, unknown>>
>;
