/**
 * Routes that live in their own auxiliary window (the desktop pet, the
 * limits strip and the menu-bar usage panel). Those windows load the same SPA as the main window, so the
 * root layout must not mount the shell there: no sidebar, no boot work, and
 * none of the global dialogs. The recovery dialog in particular opened on
 * every window at launch, so the tiny strip showed a clipped "Resume
 * downloads" button with a scrollbar until someone reloaded it.
 */
export const BARE_WINDOW_ROUTES = ["/pet", "/limits-strip", "/usage-panel", "/omni"] as const;

export function isBareWindow(pathname: string): boolean {
  const path = pathname.length > 1 ? pathname.replace(/\/+$/, "") : pathname;
  return (BARE_WINDOW_ROUTES as readonly string[]).includes(path);
}
