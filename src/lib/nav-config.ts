export interface NavItem {
  href: string;
  labelKey?: string;
  label?: string;
  icon: string;
  group: "primary" | "app" | "secondary";
  badge?: "downloads";
  order?: number;
}

export const CORE_NAV_ITEMS: NavItem[] = [
  { href: "/", labelKey: "nav.home", icon: "home", group: "primary", order: 10 },
  { href: "/downloads", labelKey: "nav.downloads", icon: "downloads", group: "primary", badge: "downloads", order: 20 },
  { href: "/llm", labelKey: "nav.llm", icon: "llm", group: "primary", order: 24 },
  { href: "/help", labelKey: "nav.help", icon: "help", group: "primary", order: 25 },
  { href: "/world", labelKey: "nav.world", icon: "world", group: "primary", order: 26 },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "app", order: 40 },
  { href: "/about", labelKey: "nav.about", icon: "about", group: "app", order: 50 },
];

