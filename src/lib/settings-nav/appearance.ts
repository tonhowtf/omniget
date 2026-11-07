import type { SettingsNavPart } from "./types";

export const appearanceNav: SettingsNavPart = {
  sections: ["theme", "colors", "comfort", "fonts"],
  sectionLabels: {
    "theme": "settings.appearance.theme",
    "colors": "settings.appearance.colors.title",
    "comfort": "settings.appearance.comfort.title",
    "fonts": "settings.appearance.fonts.title",
  },
  aliases: {
    "settings.cat_appearance": { tab: "appearance", section: null },
    "settings.appearance.theme": { tab: "appearance", section: "theme" },
    "settings.appearance.mode.system": { tab: "appearance", section: "theme" },
    "settings.appearance.mode.light": { tab: "appearance", section: "theme" },
    "settings.appearance.mode.dark": { tab: "appearance", section: "theme" },
    "settings.appearance.colors.title": { tab: "appearance", section: "colors" },
    "settings.appearance.more_themes": { tab: "appearance", section: "colors" },
    "settings.appearance.density.label": { tab: "appearance", section: "comfort" },
    "settings.appearance.reduce_motion": { tab: "appearance", section: "comfort" },
    "settings.appearance.reduce_transparency": { tab: "appearance", section: "comfort" },
    "settings.appearance.fonts.ui": { tab: "appearance", section: "fonts" },
    "settings.appearance.fonts.code": { tab: "appearance", section: "fonts" },
  },
  legacy: {
    "appearance|workspace": ["appearance", "colors"],
    "appearance|style": ["appearance", "colors"],
    "appearance|accessibility": ["appearance", "comfort"],
    "appearance|typography": ["appearance", "comfort"],
    "typography|": ["appearance", "comfort"],
  },
  searchExtra: {
    "comfort": ["settings.appearance.density.compact", "settings.appearance.density.comfortable"],
  },
};
