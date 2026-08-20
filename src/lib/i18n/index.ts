import i18n from "sveltekit-i18n";

type Payload = [payload?: Record<string, unknown>];

const config = {
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
      locale: "id",
      key: "",
      loader: async () => (await import("./id.json")).default,
    },
  ],
};

export const defaultLocale = "en";

export const { t, locale, locales, loading, loadTranslations } = new i18n<Payload>(config);
