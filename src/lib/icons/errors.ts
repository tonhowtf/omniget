import type { IconsError, IconsErrorKind } from "./types";

const KINDS: IconsErrorKind[] = ["offline", "invalid", "http", "io"];

export const ERROR_CODES = [
  "timeout",
  "unreachable",
  "interrupted",
  "client",
  "status",
  "bad_collections",
  "bad_search",
  "bad_icons",
  "query_empty",
  "query_too_long",
  "unknown_set",
  "bad_name",
  "bad_icon_id",
  "icon_too_large",
  "too_many_collections",
  "too_many_icons",
  "collection_name",
  "collection_id",
  "export_empty",
  "too_many_names",
  "no_svg",
  "no_png",
  "png_damaged",
  "not_png",
  "file_size",
  "export_too_large",
  "no_sprite",
  "file_exists",
  "no_data_dir",
  "io",
  "unknown",
] as const;

const KNOWN = new Set<string>(ERROR_CODES);

function params(raw: unknown): Record<string, string> {
  if (!raw || typeof raw !== "object") return {};
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(raw as Record<string, unknown>)) out[k] = String(v);
  return out;
}

export function errorOf(error: unknown): IconsError {
  if (error && typeof error === "object" && "kind" in error) {
    const e = error as { kind: unknown; code?: unknown; params?: unknown; message?: unknown };
    if (KINDS.includes(e.kind as IconsErrorKind)) {
      const kind = e.kind as IconsErrorKind;
      return { kind, code: typeof e.code === "string" ? e.code : kind, params: params(e.params), message: String(e.message ?? "") };
    }
  }
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
  return { kind: "io", code: "unknown", params: {}, message };
}

export function errorKey(e: IconsError): { key: string; params: Record<string, string> } {
  if (KNOWN.has(e.code)) return { key: `icons.err.${e.code}`, params: e.params };
  if (e.kind === "offline") return { key: "icons.err.unreachable", params: {} };
  if (e.kind === "http") return { key: "icons.err.bad_search", params: {} };
  if (e.kind === "invalid") return { key: "icons.err.unknown", params: {} };
  return { key: "icons.err.io", params: {} };
}
