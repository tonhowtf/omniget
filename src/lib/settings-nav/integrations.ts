import type { SettingsNavPart } from "./types";

const POWERS = ["music", "video", "tube", "read", "telegram", "washmybelly", "league"] as const;

const powerAliases = Object.fromEntries(
  POWERS.map((id) => [`settings.integrations.superpowers.${id}`, { tab: "integrations" as const, section: "superpowers" }]),
);

const powerLegacy = Object.fromEntries(
  POWERS.flatMap((id) => [
    [`integrations|${id}`, ["integrations", "superpowers"] as ["integrations", string]],
    [`advanced|${id}`, ["integrations", "superpowers"] as ["integrations", string]],
  ]),
);

export const integrationsNav: SettingsNavPart = {
  sections: ["extension", "superpowers", "pet", "rpc"],
  sectionLabels: {
    "extension": "settings.bridge.title",
    "superpowers": "nav.superpowers",
    "pet": "settings.integrations.pet.title",
    "rpc": "settings.integrations.rpc.title",
  },
  aliases: {
    "settings.integrations.rpc.title": { tab: "integrations", section: "rpc" },
    "settings.rpc.enable": { tab: "integrations", section: "rpc" },
    "settings.rpc.test": { tab: "integrations", section: "rpc" },
    "settings.bridge.title": { tab: "integrations", section: "extension" },
    "settings.bridge.pair_label": { tab: "integrations", section: "extension" },
    "settings.integrations.extension.install": { tab: "integrations", section: "extension" },
    "settings.browser_ext.title": { tab: "integrations", section: "extension" },
    "settings.bridge.endpoint_label": { tab: "integrations", section: "extension" },
    "settings.bridge.token_label": { tab: "integrations", section: "extension" },
    "settings.bridge.rotate_label": { tab: "integrations", section: "extension" },
    "nav.superpowers": { tab: "integrations", section: "superpowers" },
    ...powerAliases,
    "settings.integrations.pet.title": { tab: "integrations", section: "pet" },
    "pet.enable": { tab: "integrations", section: "pet" },
    "pet.menu.corner": { tab: "integrations", section: "pet" },
    "pet.click_through": { tab: "integrations", section: "pet" },
    "world.settings.title": { tab: "ai", section: "canvas" },
    "world.settings.enabled": { tab: "ai", section: "canvas" },
    "washmybelly.settings_quarantine_days": { href: "/washmybelly?m=receipts", labelKey: "nav.superpowers" },
    "washmybelly.settings_auto_hold": { href: "/washmybelly?m=power" },
    "league.auto_accept": { href: "/league?tab=automation" },
  },
  legacy: {
    ...powerLegacy,
    "integrations|world": ["ai", "canvas"],
    "integrations|canvas": ["ai", "canvas"],
    "integrations|extension-install": ["integrations", "extension"],
    "integrations|extension-details": ["integrations", "extension"],
    "integrations|rpc-advanced": ["integrations", "rpc"],
    "appearance|rpc": ["integrations", "rpc"],
    "dependencies|extension": ["integrations", "extension"],
    "network|extension": ["integrations", "extension"],
    "world|": ["ai", "canvas"],
    "world|pet": ["integrations", "pet"],
  },
  searchExtra: {},
};
