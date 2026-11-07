import type { SettingsCategory } from "../settings-navigation";

export type SettingAlias = { tab: SettingsCategory; section: string | null } | { href: string; labelKey?: string };

export type SettingsNavPart = {
  sections: readonly string[];
  sectionLabels: Record<string, string>;
  aliases: Record<string, SettingAlias>;
  legacy: Record<string, [SettingsCategory, string | null]>;
  searchExtra: Record<string, string[]>;
};
