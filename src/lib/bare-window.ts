const BARE_WINDOW_ROUTES = ["/pet", "/limits-strip", "/usage-panel", "/omni"] as const;

export function isBareWindow(pathname: string): boolean {
  const path = pathname.length > 1 ? pathname.replace(/\/+$/, "") : pathname;
  return (BARE_WINDOW_ROUTES as readonly string[]).includes(path);
}
