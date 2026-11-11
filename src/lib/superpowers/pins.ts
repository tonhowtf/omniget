import type { NavItem } from "$lib/nav-config";

export const MAX_RAIL_PINS = 6;

const SUPERPOWERS_ORDER = 30;

export interface Pinnable {
  id: string;
  icon: string;
  titleKey?: string;
  title?: string;
  href: string;
}

export function togglePin(pinned: string[], id: string): string[] {
  return pinned.includes(id) ? pinned.filter((entry) => entry !== id) : [...pinned, id];
}

export function pinnedNavItems(catalog: Pinnable[], pinned: string[], enabled: Record<string, boolean>): NavItem[] {
  return [...new Set(pinned)]
    .map((id) => catalog.find((entry) => entry.id === id))
    .filter((entry): entry is Pinnable => !!entry && enabled[entry.id] !== false)
    .slice(0, MAX_RAIL_PINS)
    .map((entry, index) => ({
      href: entry.href,
      ...(entry.titleKey ? { labelKey: entry.titleKey } : { label: entry.title }),
      icon: entry.icon,
      group: "app" as const,
      order: SUPERPOWERS_ORDER + (index + 1) / 10,
      pinned: true,
    }));
}
