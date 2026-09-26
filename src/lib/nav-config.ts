export interface NavItem {
  href: string;
  labelKey?: string;
  label?: string;
  icon: string;
  iconSvg?: string;
  group: "primary" | "app" | "plugins" | "secondary";
  badge?: "downloads";
  pluginId?: string;
  order?: number;
}

export const CORE_NAV_ITEMS: NavItem[] = [
  { href: "/", labelKey: "nav.home", icon: "home", group: "primary", order: 10 },
  { href: "/downloads", labelKey: "nav.downloads", icon: "downloads", group: "primary", badge: "downloads", order: 20 },
  { href: "/llm", labelKey: "nav.llm", icon: "llm", group: "primary", order: 24 },
  { href: "/help", labelKey: "nav.help", icon: "help", group: "primary", order: 25 },
  { href: "/world", labelKey: "nav.world", icon: "world", group: "primary", order: 26 },
  { href: "/tools", labelKey: "nav.tools", icon: "tools", group: "primary", order: 27 },
  { href: "/superpowers", labelKey: "nav.superpowers", icon: "superpowers", group: "app", order: 30 },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "app", order: 40 },
  { href: "/about", labelKey: "nav.about", icon: "about", group: "app", order: 50 },
];

/** Routes that live inside a hub keep the hub's sidebar item lit. */
export const NAV_ALIASES: Record<string, string[]> = {
  "/superpowers": ["/league"],
};

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
