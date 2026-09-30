// The appearance catalog. Single source: the Rust crate embeds the same file
// (`omniget-avatar/catalog/catalog_v1.json`) and hashes it; the bake script
// reads its face targets.
import raw from '../../../../src-tauri/omniget-avatar/catalog/catalog_v1.json';

export interface Named { id: string; hex?: string }
export interface HairStyle extends Named {
  shape: 'none' | 'cap' | 'crest';
  thickness?: number; cut?: number; fringe?: number; bumps?: number;
  tail?: number; ponytail?: number; bun?: number; braids?: number;
}
export interface Piece {
  id: string; cover: string[]; offset: number; license?: string;
  full_body?: boolean; skirt?: number; collar?: boolean; apron?: boolean; shape?: string;
}
export interface Catalog {
  schema: string; version: number;
  phenotypes: string[];
  face_targets: { id: string; labels: string[] }[];
  skin_tones: Named[]; hair_styles: HairStyle[]; hair_colors: Named[]; eye_colors: Named[];
  brows: string[]; beards: string[];
  tops: Piece[]; bottoms: Piece[]; shoes: Piece[]; accessories: Piece[];
  cloth_colors: string[];
  palettes: { id: string; colors: number[] }[];
  gaits: string[]; idles: string[]; moods: string[];
  words: Record<string, Record<string, number>>;
}

export const catalog = raw as unknown as Catalog;

/** Same FNV-1a 64 the Rust side uses, over the JSON text as served. */
export function fnv1a64Hex(bytes: Uint8Array): string {
  let h = 0xcbf29ce484222325n;
  const p = 0x100000001b3n;
  for (const b of bytes) h = BigInt.asUintN(64, (h ^ BigInt(b)) * p);
  return h.toString(16).padStart(16, '0');
}

export function hexColor(hex: string): [number, number, number] {
  const h = hex.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as [number, number, number];
}
