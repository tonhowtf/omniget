export type IconSet = {
  prefix: string;
  name: string;
  total: number;
  author: string | null;
  author_url: string | null;
  license: string | null;
  license_spdx: string | null;
  license_url: string | null;
  category: string | null;
  palette: boolean;
  height: number | null;
  samples: string[];
  tags: string[];
};

export type IconData = {
  name: string;
  body: string;
  width: number;
  height: number;
  left: number;
  top: number;
  rotate: number;
  h_flip: boolean;
  v_flip: boolean;
};

export type CollectionsResult = { sets: IconSet[]; offline: boolean; fetched_at: number };
export type SearchResult = { icons: string[]; capped: boolean; more: string[]; next_start: number; trimmed: number; offline: boolean };
export type IconsResult = { prefix: string; icons: IconData[]; missing: string[]; offline: boolean };

export type SavedIcon = {
  prefix: string;
  icon: IconData;
  set_name: string | null;
  license: string | null;
  license_spdx: string | null;
  added_at: number;
};

export type NamedCollection = { id: string; name: string; created_at: number; icons: SavedIcon[] };
export type Library = { version: number; favorites: SavedIcon[]; collections: NamedCollection[] };

export type IconsErrorKind = "offline" | "invalid" | "http" | "io";
export type IconsError = { kind: IconsErrorKind; code: string; params: Record<string, string>; message: string };

export type SavedFile = { path: string; count: number };
export type ExportFile = { name: string; svg: string };
