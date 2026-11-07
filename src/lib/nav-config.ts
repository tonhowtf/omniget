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
  { href: "/help", labelKey: "nav.help", icon: "help", group: "primary", order: 25 },
  { href: "/world", labelKey: "nav.world", icon: "world", group: "primary", order: 26 },
  { href: "/superpowers", labelKey: "nav.superpowers", icon: "superpowers", group: "app", order: 30 },
  { href: "/settings", labelKey: "nav.settings", icon: "settings", group: "app", order: 40 },
  { href: "/community", labelKey: "nav.community", icon: "community", group: "app", order: 50 },
];

export const NAV_ALIASES: Record<string, string[]> = {
  "/superpowers": ["/league", "/music", "/washmybelly", "/video", "/global", "/telegram", "/youtube", "/read", "/converter", "/pdf", "/photos", "/cutout", "/palette", "/fonts", "/icons", "/motion", "/board", "/components", "/capture", "/studio", "/dictation", "/whatsapp", "/discord", "/x", "/instagram", "/tiktok"],
};
