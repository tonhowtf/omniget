export interface NavItem {
  href: string;
  labelKey?: string;
  label?: string;
  icon: string;
  iconSvg?: string;
  group: "primary" | "secondary";
  badge?: "downloads";
  pluginId?: string;
  order?: number;
}

export const CORE_NAV_ITEMS: NavItem[] = [
  { href: "/", labelKey: "nav.home", icon: "home", group: "primary", order: 10 },
  { href: "/downloads", labelKey: "nav.downloads", icon: "downloads", group: "primary", badge: "downloads", order: 20 },
  { href: "/marketplace", labelKey: "nav.marketplace", icon: "marketplace", group: "primary", order: 30 },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "primary", order: 40 },
  { href: "/about", labelKey: "nav.about", icon: "about", group: "secondary", order: 999 },
];
