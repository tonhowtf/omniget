// Which settings fields currently show their own inline SettingStatus. A failed save
// is announced once: inline (role=alert, next to the control) when a status is mounted
// for every changed field, and by toast only when some field has no inline status.

const surfaces = new Map<string, number>();

/** Called by SettingStatus while mounted with a `path`; returns the unregister. */
export function registerStatusSurface(path: string): () => void {
  surfaces.set(path, (surfaces.get(path) ?? 0) + 1);
  let active = true;
  return () => {
    if (!active) return;
    active = false;
    const count = (surfaces.get(path) ?? 1) - 1;
    if (count <= 0) surfaces.delete(path);
    else surfaces.set(path, count);
  };
}

/** A status for `path` itself or for one of its prefixes (e.g. "proxy" covers "proxy.host"). */
function hasStatusSurface(path: string): boolean {
  for (const surface of surfaces.keys()) {
    if (path === surface || path.startsWith(`${surface}.`)) return true;
  }
  return false;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function leafPaths(patch: Record<string, unknown>, prefix = ""): string[] {
  const paths: string[] = [];
  for (const [key, value] of Object.entries(patch)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (isPlainObject(value) && Object.keys(value).length > 0) paths.push(...leafPaths(value, path));
    else paths.push(path);
  }
  return paths;
}

/** True when every field of `patch` already reports its failure inline. */
export function failureShownInline(patch: Record<string, unknown>): boolean {
  const paths = leafPaths(patch);
  return paths.length > 0 && paths.every(hasStatusSurface);
}
