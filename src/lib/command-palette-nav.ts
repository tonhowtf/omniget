import type { NavItem } from "$lib/nav-config";
import type { Superpower } from "$lib/superpowers/catalog";

export interface PaletteNavEntry {
  id: string;
  href: string;
  labelKey?: string;
  label?: string;
  keywords: string;
}

export function paletteNavEntries(core: NavItem[], powers: Superpower[], enabled: Record<string, boolean>): PaletteNavEntry[] {
  const coreHrefs = new Set(core.map((item) => item.href));
  return [
    ...core.map((item) => ({
      id: `nav-${item.href.slice(1) || "home"}`,
      href: item.href,
      labelKey: item.labelKey,
      label: item.label,
      keywords: item.keywords ?? "",
    })),
    ...powers
      .filter((entry) => enabled[entry.id] !== false && !coreHrefs.has(entry.href))
      .map((entry) => ({
        id: `nav-${entry.id}`,
        href: entry.href,
        labelKey: entry.titleKey,
        label: entry.title,
        keywords: entry.keywords,
      })),
  ];
}
