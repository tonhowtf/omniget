import { SUPERPOWERS } from "$lib/superpowers/catalog";

export interface NavItem {
  href: string;
  labelKey?: string;
  label?: string;
  icon: string;
  group: "primary" | "app" | "secondary";
  badge?: "downloads";
  order?: number;
  pinned?: boolean;
}

export const CORE_NAV_ITEMS: NavItem[] = [
  { href: "/", labelKey: "nav.home", icon: "home", group: "primary", order: 10 },
  { href: "/downloads", labelKey: "nav.downloads", icon: "downloads", group: "primary", badge: "downloads", order: 20 },
  { href: "/help", labelKey: "nav.help", icon: "help", group: "primary", order: 25 },
  { href: "/world", labelKey: "nav.world", icon: "world", group: "primary", order: 26 },
  { href: "/superpowers", labelKey: "nav.superpowers", icon: "superpowers", group: "app", order: 30 },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "app", order: 40 },
  { href: "/community", labelKey: "nav.community", icon: "community", group: "app", order: 50 },
];

export const NAV_ALIASES: Record<string, string[]> = {
  "/superpowers": SUPERPOWERS.map((entry) => entry.href),
};

function onRoute(path: string, href: string): boolean {
  return path === href || (href !== "/" && path.startsWith(`${href}/`));
}

export function activeNavHref(items: NavItem[], path: string): string | null {
  const direct = items.find((item) => onRoute(path, item.href));
  if (direct) return direct.href;
  const viaAlias = items.find((item) => (NAV_ALIASES[item.href] ?? []).some((alias) => onRoute(path, alias)));
  return viaAlias?.href ?? null;
}
