export type SourceKind = "image" | "video";

export type SourceInfo = {
  path: string;
  name: string;
  kind: SourceKind;
  width: number;
  height: number;
  duration_ms: number;
};

export type Inspected = {
  path: string;
  name: string;
  info: SourceInfo | null;
  error: string | null;
};

export type ModelInfo = {
  id: string;
  name: string;
  bytes: number;
  license: string;
  source: string;
  downloaded: boolean;
};

export type Status = {
  runtime_installed: boolean;
  runtime_downloadable: boolean;
  models: ModelInfo[];
  refine: ModelInfo;
};

export type Click = { x: number; y: number; keep: boolean };

export type Edge = { feather: number; shift: number };

export type Background = { kind: "none" } | { kind: "color"; color: string } | { kind: "image"; path: string };

export type OutputFormat = "png" | "webp";

export type Preview = {
  before: string;
  after: string;
  width: number;
  height: number;
};

export type CutItem = { path: string; frame_ms: number | null; clicks: Click[] };

export type CutOptions = {
  model: string;
  edge: Edge;
  background: Background;
  format: OutputFormat;
  out_dir: string | null;
  overwrite: boolean;
};

export type CutOutcome = {
  input: string;
  frame_ms: number | null;
  output: string | null;
  size_bytes: number;
  ok: boolean;
  error: string | null;
};

export type ProgressState = "queued" | "running" | "done" | "failed";

export type ProgressEvent = {
  batch_id: string;
  index: number;
  state: ProgressState;
  outcome?: CutOutcome | null;
};

export type InstallProgress = {
  model: string;
  id: string;
  stage: string;
  done: number;
  total: number | null;
};
