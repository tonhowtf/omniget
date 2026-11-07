import { SETTING_ALIASES, SECTION_LABEL_KEYS, CATEGORY_LABEL_KEYS, SETTINGS_CATEGORIES, settingsHref, type SettingsCategory } from "./settings-navigation";
import { NAV_PARTS } from "./settings-nav";

export type { SettingsCategory } from "./settings-navigation";

export type SettingSearchEntry = {
  key: string;
  category: SettingsCategory;
  section: string;
  sectionKey: string;
  href?: string;
};

const EXTRA: [SettingsCategory, string, string[]][] = SETTINGS_CATEGORIES.flatMap((category) =>
  Object.entries(NAV_PARTS[category].searchExtra).map(([section, keys]) => [category, section, keys] as [SettingsCategory, string, string[]]),
);

function sectionKeyFor(category: SettingsCategory, section: string): string {
  return section ? SECTION_LABEL_KEYS[section] ?? CATEGORY_LABEL_KEYS[category] : CATEGORY_LABEL_KEYS[category];
}

function buildIndex(): SettingSearchEntry[] {
  const entries: SettingSearchEntry[] = [];
  const seen = new Set<string>();
  for (const [key, alias] of Object.entries(SETTING_ALIASES)) {
    if (seen.has(key)) continue;
    seen.add(key);
    if ("href" in alias) {
      entries.push({ key, category: "integrations", section: "", sectionKey: alias.labelKey ?? "nav.superpowers", href: alias.href });
    } else {
      const section = alias.section ?? "";
      entries.push({ key, category: alias.tab, section, sectionKey: sectionKeyFor(alias.tab, section) });
    }
  }
  for (const [category, section, keys] of EXTRA) {
    for (const key of keys) {
      if (seen.has(key)) continue;
      seen.add(key);
      entries.push({ key, category, section, sectionKey: sectionKeyFor(category, section) });
    }
  }
  return entries;
}

const SETTINGS_SEARCH_INDEX: SettingSearchEntry[] = buildIndex();

function normalizeSettingsQuery(text: string): string {
  return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLocaleLowerCase().trim();
}

export function searchSettings(query: string, translate: (key: string) => string): SettingSearchEntry[] {
  const terms = normalizeSettingsQuery(query).split(/\s+/).filter(Boolean);
  if (!terms.length) return [];
  return SETTINGS_SEARCH_INDEX.filter(entry => {
    const parentTitle = entry.key.replace(/\.[^.]+$/, ".title");
    const keys = [entry.key, entry.sectionKey, parentTitle, `${entry.key}_desc`, `${entry.key}_hint`];
    const text = normalizeSettingsQuery(keys.map(key => {
      const value = translate(key);
      return value === key ? "" : value;
    }).join(" "));
    return terms.every(term => text.includes(term));
  });
}

export function settingsEntryUrl(entry: SettingSearchEntry): string {
  return entry.href ?? settingsHref(entry.category, entry.section || null, entry.key);
}

export function parseSettingNumber(value: string, min: string, max: string): number | null {
  if (!value.trim()) return null;
  const number = Number(value);
  if (!Number.isFinite(number) || !Number.isInteger(number)) return null;
  if (min !== "" && number < Number(min)) return null;
  if (max !== "" && number > Number(max)) return null;
  return number;
}
