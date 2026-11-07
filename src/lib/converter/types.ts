export type Category = "data" | "document" | "image" | "cad" | "audio" | "video";

export type Engine = "native" | "ffmpeg" | "pandoc" | "imagemagick" | "libreoffice";

export type ExternalTool = Exclude<Engine, "native">;

export type ToolStatus = Record<ExternalTool, boolean>;

export type FormatInfo = {
  id: string;
  label: string;
  category: Category;
  extensions: string[];
};

export type RouteStep = {
  from: string;
  to: string;
  engine: Engine;
  lossy: boolean;
};

export type Route = {
  steps: RouteStep[];
  lossy: boolean;
  engines: Engine[];
};

export type TargetOption = {
  format: string;
  label: string;
  category: Category;
  route: Route;
  missing_tool?: Engine | null;
};

export type InspectResult = {
  path: string;
  name: string;
  size_bytes: number;
  format: FormatInfo | null;
  targets: TargetOption[];
  error: string | null;
};

export type VectorPreset = "colors" | "bw" | "lineart";

export type ConvertOptions = {
  pretty?: boolean;
  sort_keys?: boolean;
  quality?: number;
  max_width?: number;
  max_height?: number;
  overwrite?: boolean;
  vector_preset?: VectorPreset;
};

export type ConvertOutcome = {
  input: string;
  output: string | null;
  size_bytes: number;
  route: Route | null;
  ok: boolean;
  error: string | null;
  skipped: boolean;
};

export type ProgressState = "queued" | "running" | "done" | "failed" | "skipped";

export type ProgressEvent = {
  batch_id: string;
  index: number;
  path: string;
  percent: number;
  state: ProgressState;
  outcome?: ConvertOutcome | null;
};

export type RunItem = { path: string; target: string };
