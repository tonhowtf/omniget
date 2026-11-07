export type PdfInfo = {
  path: string;
  name: string;
  size_bytes: number;
  pages: number;
  error: string | null;
};

export type PageSpec =
  | { kind: "page"; source: number; page: number; rotate: number }
  | { kind: "blank"; width: number; height: number };

export type OutputPlan = {
  path: string;
  pages: PageSpec[];
};

export type SavedFile = {
  path: string;
  size_bytes: number;
};

export type Preset = "light" | "balanced" | "strong";

export type CompressOutcome = {
  path: string | null;
  before_bytes: number;
  after_bytes: number;
  images: number;
};

export type PageSize = "fit" | "a4" | "letter";

export type ImageLayout = {
  size: PageSize;
  margin: boolean;
};

export type ProgressEvent = {
  job_id: string;
  done: number;
  total: number;
};

export type PdfFailure = {
  code: string;
  message: string;
};
