const MAP: Record<string, string> = {
  en: "en",
  pt: "pt-BR",
  "pt-br": "pt-BR",
  "pt-pt": "pt-PT",
  es: "es-ES",
  fr: "fr-FR",
  it: "it-IT",
  el: "el-GR",
  ru: "ru-RU",
  ja: "ja-JP",
  zh: "zh-CN",
  "zh-cn": "zh-CN",
  "zh-hans": "zh-CN",
  "zh-tw": "zh-TW",
  "zh-hant": "zh-TW",
  "zh-hk": "zh-TW",
  fa: "fa-IR",
};

export function excalidrawLang(appLocale: string | null | undefined): string {
  if (!appLocale) return "en";
  const key = appLocale.trim().replace(/_/g, "-").toLowerCase();
  if (MAP[key]) return MAP[key];
  const base = key.split("-")[0];
  return MAP[base] ?? "en";
}
