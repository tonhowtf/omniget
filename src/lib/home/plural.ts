import { get } from "svelte/store";
import { locale } from "$lib/i18n";

const rulesCache = new Map<string, Intl.PluralRules>();

export function pluralKey(base: string, count: number): string {
  const lang = get(locale) || "en";
  let rules = rulesCache.get(lang);
  if (!rules) {
    try {
      rules = new Intl.PluralRules(lang);
    } catch {
      rules = new Intl.PluralRules("en");
    }
    rulesCache.set(lang, rules);
  }
  return `${base}_${rules.select(count) === "one" ? "one" : "other"}`;
}

export function splitLink(text: string): { before: string; link: string; after: string } {
  const m = /^(.*?)\[(.+?)\](.*)$/s.exec(text);
  if (!m) return { before: text, link: "", after: "" };
  return { before: m[1], link: m[2], after: m[3] };
}
