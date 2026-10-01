import type { TranslationBag } from "./tray-strings";

const USAGE_TRAY_MENU_KEYS = [
  "accounts",
  "no_accounts",
  "icon_shows",
  "shows_session",
  "shows_weekly",
  "shows_busiest",
  "percent_text",
  "colored",
  "thresholds",
  "threshold_item",
  "open_claude",
  "add_account",
  "open_app",
  "refresh",
  "hide",
  "tooltip",
  "toggle",
  "default_login",
  "session",
  "weekly",
] as const;

export function usageTrayStrings(bag: TranslationBag, locale: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const key of USAGE_TRAY_MENU_KEYS) {
    const flat = `usage_tray.menu.${key}`;
    const active = bag?.[locale]?.[flat];
    const fallback = bag?.en?.[flat];
    if (typeof active === "string" && active.length > 0) out[key] = active;
    else if (typeof fallback === "string" && fallback.length > 0) out[key] = fallback;
  }
  return out;
}
