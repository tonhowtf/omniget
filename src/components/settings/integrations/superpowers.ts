export const POWER_IDS = ["music", "video", "tube", "read", "telegram", "washmybelly", "league"] as const;

export type PowerId = (typeof POWER_IDS)[number];

type PowerSettings = Partial<Record<PowerId, { enabled?: boolean } | null | undefined>>;

export function powerEnabled(settings: PowerSettings | null | undefined, power: PowerId): boolean {
  return settings?.[power]?.enabled ?? true;
}

export function powerPatch(settings: PowerSettings | null | undefined, power: PowerId, enabled: boolean): Record<string, unknown> {
  if (power === "tube") return { tube: { ...(settings?.tube ?? {}), enabled } };
  return { [power]: { enabled } };
}
