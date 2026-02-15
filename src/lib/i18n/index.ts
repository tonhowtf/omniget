import i18n from "sveltekit-i18n";

type Payload = [payload?: Record<string, unknown>];

const config = {
  loaders: [
    {
      locale: "pt",
      key: "",
      loader: async () => (await import("./pt.json")).default,
    },
    {
      locale: "en",
      key: "",
      loader: async () => (await import("./en.json")).default,
    },
  ],
};

export const defaultLocale = "pt";

export const { t, locale, locales, loading, loadTranslations } = new i18n<Payload>(config);
