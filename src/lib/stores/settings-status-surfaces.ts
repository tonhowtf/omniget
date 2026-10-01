const surfaces = new Map<string, number>();

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

export function failureShownInline(patch: Record<string, unknown>): boolean {
  const paths = leafPaths(patch);
  return paths.length > 0 && paths.every(hasStatusSurface);
}
