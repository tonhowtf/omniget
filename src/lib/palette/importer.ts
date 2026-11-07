import { importText, type ImportResult } from "./export";
import type { PaletteNamer } from "./project";

export type FileImport = ImportResult | { ok: false; reason: "big" | "unreadable" };

export const MAX_IMPORT_BYTES = 8 * 1024 * 1024;

export async function importFile(file: File, untitled: string, paletteName: PaletteNamer): Promise<FileImport> {
  if (file.size > MAX_IMPORT_BYTES) return { ok: false, reason: "big" };
  const text = await file.text().catch(() => null);
  if (text === null) return { ok: false, reason: "unreadable" };
  const stem = file.name.replace(/(\.tokens|\.palette)?\.json$/i, "") || untitled;
  return importText(text, stem, paletteName);
}
