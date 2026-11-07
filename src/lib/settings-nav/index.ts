import type { SettingsCategory } from "../settings-navigation";
import type { SettingsNavPart } from "./types";
import { generalNav } from "./general";
import { appearanceNav } from "./appearance";
import { downloadsNav } from "./downloads";
import { aiNav } from "./ai";
import { integrationsNav } from "./integrations";
import { advancedNav } from "./advanced";

export const NAV_PARTS: Record<SettingsCategory, SettingsNavPart> = {
  general: generalNav,
  appearance: appearanceNav,
  downloads: downloadsNav,
  ai: aiNav,
  integrations: integrationsNav,
  advanced: advancedNav,
};
