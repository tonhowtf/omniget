export type TranslationBag = Record<string, Record<string, unknown>> | undefined;

type TrayStrings = {
  quit: string;
  downloadsNone: string;
  downloadsActive: string;
  channels: string;
  tooltipActive: string;
  tooltipSpeed: string;
  quickTools: string;
  awakeOn: string;
  awakeUntil: string;
};

const KEYS: Record<keyof TrayStrings, string> = {
  quit: "tray.quit",
  downloadsNone: "tray.downloads_none",
  downloadsActive: "tray.downloads_active",
  channels: "settings.channels.tray_header",
  tooltipActive: "tray.tooltip_active",
  tooltipSpeed: "tray.tooltip_speed",
  quickTools: "tray.quick_tools",
  awakeOn: "tray.awake_on",
  awakeUntil: "tray.awake_until",
};

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
    quickTools: pick(KEYS.quickTools),
    awakeOn: pick(KEYS.awakeOn),
    awakeUntil: pick(KEYS.awakeUntil),
  };
}
