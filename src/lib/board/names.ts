export const EXTENSION = "excalidraw";
export const MAX_NAME_CHARS = 120;

export type NameError = "empty" | "invalid" | "too_long" | "exists";
export type NameCheck = { ok: true; name: string } | { ok: false; error: NameError };

const FORBIDDEN = /[/\\:*?"<>|\u0000-\u001f\u007f]/;
const RESERVED = new Set(["CON", "PRN", "AUX", "NUL", ...[1, 2, 3, 4, 5, 6, 7, 8, 9].flatMap((n) => [`COM${n}`, `LPT${n}`])]);

export function stripExtension(raw: string): string {
  const trimmed = raw.trim();
  const suffix = `.${EXTENSION}`;
  return trimmed.toLowerCase().endsWith(suffix) ? trimmed.slice(0, -suffix.length).trimEnd() : trimmed;
}

export function checkName(raw: string, taken: readonly string[] = [], current?: string): NameCheck {
  const name = stripExtension(raw);
  if (!name || name === "." || name === "..") return { ok: false, error: "empty" };
  if (FORBIDDEN.test(name) || name.startsWith(".") || name.endsWith(".")) return { ok: false, error: "invalid" };
  if (RESERVED.has(name.split(".")[0].toUpperCase())) return { ok: false, error: "invalid" };
  if ([...name].length > MAX_NAME_CHARS) return { ok: false, error: "too_long" };
  const lower = name.toLowerCase();
  const others = current === undefined ? taken : taken.filter((t) => t !== current);
  if (others.some((t) => t.toLowerCase() === lower)) return { ok: false, error: "exists" };
  return { ok: true, name };
}

export function sanitizeName(raw: string, fallback = "board"): string {
  const cleaned = stripExtension(raw)
    .replace(/[/\\:*?"<>|\u0000-\u001f\u007f]+/g, " ")
    .replace(/\s+/g, " ")
    .replace(/^[.\s]+|[.\s]+$/g, "");
  const clipped = [...cleaned].slice(0, MAX_NAME_CHARS).join("").trim();
  const check = checkName(clipped);
  return check.ok ? check.name : fallback;
}

export function fileStem(path: string): string {
  const base = path.split(/[/\\]/).pop() ?? path;
  return stripExtension(base);
}
