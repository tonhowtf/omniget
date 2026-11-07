export function isAbsolutePath(p: string): boolean {
  const s = p.trim();
  return s.startsWith("/") || /^[A-Za-z]:[\\/]/.test(s) || s.startsWith("\\\\");
}

export function shortPath(p: string, max = 48): string {
  if (p.length <= max) return p;
  const keep = Math.max(4, Math.floor((max - 1) / 2));
  return `${p.slice(0, keep)}…${p.slice(p.length - keep)}`;
}
