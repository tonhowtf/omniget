export type Photo = {
  path: string;
  name: string;
  folder: string;
  format: string;
  size: number;
  modified_at: number;
  taken_at: number | null;
  width: number;
  height: number;
  thumb: string | null;
  error: string | null;
  edited: boolean;
};

export type Folder = { path: string; exists: boolean };

export type Folders = { folders: Folder[]; chosen: boolean };

export type Stage = "idle" | "scanning" | "indexing" | "done" | "cancelled" | "failed";

export type IndexProgress = {
  stage: Stage;
  found: number;
  total: number;
  done: number;
  failed: number;
  error: string | null;
};

export type DupKind = "exact" | "similar";

export type DupGroup = { kind: DupKind; keep: string; paths: string[]; bytes: number };

export type RemoveMode = "trash" | "delete";

export type FileReport = { done: string[]; moved_to: string[]; failed: { path: string; error: string }[] };

export type Crop = { x: number; y: number; w: number; h: number };

export type Edits = {
  exposure: number;
  contrast: number;
  saturation: number;
  temperature: number;
  rotate: number;
  crop: Crop | null;
};

export type Preview = { src: string; width: number; height: number; full_width: number; full_height: number };

export type ExportFormat = "jpeg" | "png" | "webp" | "tiff";

export type ExportOptions = {
  format: ExportFormat;
  quality: number | null;
  max_size: number | null;
  out_dir: string | null;
  apply_edits: boolean;
};

export type ExportOutcome = { input: string; output: string | null; size_bytes: number; ok: boolean; error: string | null };

export type ExportProgress = {
  batch_id: string;
  index: number;
  path: string;
  state: "queued" | "running" | "done" | "failed";
  outcome?: ExportOutcome;
};
