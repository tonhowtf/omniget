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
  keywords?: string;
}

export const CORE_NAV_ITEMS: NavItem[] = [
  { href: "/", labelKey: "nav.home", icon: "home", group: "primary", order: 10, keywords: "index main start inicio início" },
  { href: "/downloads", labelKey: "nav.downloads", icon: "downloads", group: "primary", badge: "downloads", order: 20, keywords: "queue fila active history histórico" },
  { href: "/superpowers", labelKey: "nav.superpowers", icon: "superpowers", group: "app", order: 30, keywords: "tools ferramentas superpowers superpoderes extras" },
  { href: "/world", labelKey: "nav.world", icon: "world", group: "app", order: 35, keywords: "canvas world agents agentes casa house" },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "app", order: 40, keywords: "preferences options config configurações" },
  { href: "/help", labelKey: "nav.help", icon: "help", group: "app", order: 45, keywords: "help ajuda guias guides docs" },
  { href: "/community", labelKey: "nav.community", icon: "community", group: "app", order: 50, keywords: "community comunidade chat discord info about sobre version changelog project" },
];

const BEYOND_DOWNLOADS = new Set(["/superpowers", "/world"]);

export function visibleCoreNav(opts: { downloadsOnly?: boolean; worldEnabled?: boolean }): NavItem[] {
  return CORE_NAV_ITEMS.filter((item) => {
    if (item.href === "/world" && opts.worldEnabled === false) return false;
    return !(opts.downloadsOnly && BEYOND_DOWNLOADS.has(item.href));
  });
}

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
