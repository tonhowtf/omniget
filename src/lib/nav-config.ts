export interface NavItem {
  href: string;
  labelKey?: string;
  label?: string;
  icon: string;
  iconSvg?: string;
  group: "primary" | "app" | "plugins" | "secondary";
  badge?: "downloads" | "omnidisc";
  pluginId?: string;
  order?: number;
}

export const CORE_NAV_ITEMS: NavItem[] = [
  { href: "/", labelKey: "nav.home", icon: "home", group: "primary", order: 10 },
  { href: "/downloads", labelKey: "nav.downloads", icon: "downloads", group: "primary", badge: "downloads", order: 20 },
  { href: "/omnidisc", labelKey: "nav.omnidisc", icon: "chat", group: "primary", badge: "omnidisc", order: 25 },
  { href: "/tools", labelKey: "nav.tools", icon: "tools", group: "primary", order: 27 },
  { href: "/marketplace", labelKey: "nav.marketplace", icon: "marketplace", group: "app", order: 30 },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "app", order: 40 },
  { href: "/about", labelKey: "nav.about", icon: "about", group: "app", order: 50 },
];

export function pluginIconForRoute(route: string): string {
  if (route.startsWith("/courses")) return "courses";
  if (route.startsWith("/convert")) return "convert";
  if (route.startsWith("/telegram")) return "telegram";
  if (route.startsWith("/study/music")) return "music";
  if (route.startsWith("/study")) return "study";
  if (route.includes("/library")) return "library";
  if (route.startsWith("/misc")) return "misc";
  return "plugin";
}
