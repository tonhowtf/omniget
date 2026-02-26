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
      locale: "pt",
      key: "",
      loader: async () => (await import("./pt.json")).default,
    },
  ],
};

export const defaultLocale = "en";

export const { t, locale, locales, loading, loadTranslations } = new i18n<Payload>(config);
