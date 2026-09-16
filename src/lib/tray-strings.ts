/**
 * Strings for the native tray menu.
 *
 * The tray is built in Rust and cannot reach `$t`, so the frontend pushes what it
 * needs through `sync_tray_strings`. These values must come from the loaded locale
 * data (`rawTranslations`) and **not** from `$t`: the default parser substitutes
 * every `{{placeholder}}` and drops the ones it has no payload for, so
 * `$t("tray.downloads_active")` turns "Downloads: {{count}} active" into
 * "Downloads:  active". The Rust side would then have nothing left to fill and the
 * tray would lose the download count and the speed in every language, including
 * English, from the first sync. `tray-strings.test.ts` guards the placeholders.
 */

/** `{ locale: { "flat.key": value } }`, the shape `translationStore` holds. */
export type TranslationBag = Record<string, Record<string, unknown>> | undefined;

// A type alias (not an interface) on purpose: Tauri's `invoke` wants
// `Record<string, unknown>`, and TypeScript only gives an implicit index
// signature to object type aliases.
export type TrayStrings = {
  quit: string;
  downloadsNone: string;
  downloadsActive: string;
  channels: string;
  tooltipActive: string;
  tooltipSpeed: string;
};

const KEYS: Record<keyof TrayStrings, string> = {
  quit: "tray.quit",
  downloadsNone: "tray.downloads_none",
  downloadsActive: "tray.downloads_active",
  // The channels submenu title comes from settings.channels.tray_header — the same
  // key sync_channels_tray sends — so a locale change can never overwrite a
  // synchronized header with a different string.
  channels: "settings.channels.tray_header",
  tooltipActive: "tray.tooltip_active",
  tooltipSpeed: "tray.tooltip_speed",
};

/**
 * Builds the payload for `sync_tray_strings` out of the flat translation bag.
 * Falls back to English, then to the key itself, exactly like `$t` does.
 */
export function trayStrings(bag: TranslationBag, locale: string): TrayStrings {
  const pick = (key: string): string => {
    const active = bag?.[locale]?.[key];
    if (typeof active === "string" && active.length > 0) return active;
    const fallback = bag?.en?.[key];
    if (typeof fallback === "string" && fallback.length > 0) return fallback;
    return key;
  };

  return {
    quit: pick(KEYS.quit),
    downloadsNone: pick(KEYS.downloadsNone),
    downloadsActive: pick(KEYS.downloadsActive),
    channels: pick(KEYS.channels),
    tooltipActive: pick(KEYS.tooltipActive),
    tooltipSpeed: pick(KEYS.tooltipSpeed),
  };
}
