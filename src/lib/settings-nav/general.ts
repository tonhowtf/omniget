import type { SettingsNavPart } from "./types";

export const generalNav: SettingsNavPart = {
  sections: ["startup", "closing", "language", "profile", "more", "guidance"],
  sectionLabels: {
    "language": "settings.appearance.language",
    "startup": "settings.general.on_start_title",
    "closing": "settings.general.on_close_title",
    "more": "settings.general.more_title",
    "profile": "profile.title",
    "guidance": "settings.general.reset_hints",
  },
  aliases: {
    "settings.general.start_with_system": { tab: "general", section: "startup" },
    "settings.general.start_minimized": { tab: "general", section: "startup" },
    "settings.general.exit_on_close": { tab: "general", section: "closing" },
    "settings.general.run_in_background": { tab: "general", section: "closing" },
    "settings.appearance.language": { tab: "general", section: "language" },
    "settings.general.reset_hints": { tab: "general", section: "guidance" },
    "settings.general.reopen_guide": { tab: "general", section: "more" },
    "settings.general.reopen_guide_desc": { tab: "general", section: "more" },
    "profile.title": { tab: "general", section: "profile" },
    "profile.desc": { tab: "general", section: "profile" },
  },
  legacy: {
    "profile|": ["general", "profile"],
  },
  searchExtra: {
  },
};
