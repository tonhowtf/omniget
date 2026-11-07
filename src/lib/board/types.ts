export type BoardEntry = {
  name: string;
  path: string;
  modified_ms: number;
  size: number;
  has_thumb: boolean;
};

export type BoardLibrary = {
  folder: string;
  is_default: boolean;
  boards: BoardEntry[];
};

export type BoardFile = {
  path: string;
  name: string;
  contents: string;
  external: boolean;
  modified_ms: number;
};

export type SavedInfo = {
  modified_ms: number;
  size: number;
};

export type Deleted = {
  method: "os" | "folder";
  restore_path: string | null;
};

export type ExportKind = "png" | "svg" | "excalidraw";

export type ExportOptions = {
  scale: 1 | 2 | 3;
  background: boolean;
  dark: boolean;
};

export type SortMode = "recent" | "name";
