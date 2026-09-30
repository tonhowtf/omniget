// Guide entries of both faces. `root` entries reset the face's stack.
import type { Route } from "$lib/tube/ui.svelte";

export type NavEntry = {
  id: string;
  /** i18n key */
  label: string;
  icon: string;
  /** Filled variant for the active entry, when TIcon has one. */
  iconOn?: string;
  route?: Route;
  /** App route outside /youtube (Downloads). */
  href?: string;
  root?: boolean;
};

export const VIDEO_MAIN: NavEntry[] = [
  { id: "home", label: "tube.shell.nav_home", icon: "home", iconOn: "home-fill", route: { name: "home" }, root: true },
  { id: "shorts", label: "tube.shell.nav_shorts", icon: "shorts", iconOn: "shorts-fill", route: { name: "shorts" }, root: true },
  { id: "subscriptions", label: "tube.shell.nav_subscriptions", icon: "subscriptions", iconOn: "subscriptions-fill", route: { name: "subscriptions" }, root: true },
  { id: "library", label: "tube.shell.nav_you", icon: "you", iconOn: "you-fill", route: { name: "library" }, root: true },
];

export const VIDEO_YOU: NavEntry[] = [
  { id: "history", label: "tube.shell.nav_history", icon: "history", route: { name: "history" } },
  { id: "playlists", label: "tube.shell.nav_playlists", icon: "playlist", route: { name: "library" } },
  { id: "wl", label: "tube.shell.nav_watch_later", icon: "clock", route: { name: "playlist", id: "WL" } },
  { id: "ll", label: "tube.shell.nav_liked", icon: "thumbs-up", iconOn: "thumbs-up-fill", route: { name: "playlist", id: "LL" } },
  { id: "downloads", label: "tube.shell.nav_downloads", icon: "download", href: "/downloads" },
];

export const MUSIC_MAIN: NavEntry[] = [
  { id: "m_home", label: "tube.shell.nav_home", icon: "home", iconOn: "home-fill", route: { name: "m_home" }, root: true },
  { id: "m_explore", label: "tube.shell.nav_explore", icon: "explore", iconOn: "explore-fill", route: { name: "m_explore" }, root: true },
  { id: "m_library", label: "tube.shell.nav_library", icon: "library", route: { name: "m_library" }, root: true },
];

export const FOOT: NavEntry[] = [
  { id: "settings", label: "tube.shell.settings", icon: "settings", route: { name: "settings" } },
  { id: "import", label: "tube.shell.import", icon: "import", route: { name: "import" } },
];

/** Which entry is lit for the current route. */
export function activeId(r: Route): string {
  switch (r.name) {
    case "playlist":
      return r.id === "WL" ? "wl" : r.id === "LL" ? "ll" : "";
    case "m_playlist":
      return `mpl:${r.id}`;
    case "channel":
      return `ch:${r.id}`;
    default:
      return r.name;
  }
}
