export type LicenseClass = "open" | "other";

export type Axis = { tag: string; min: number; default: number; max: number };

export type InstalledStyle = { name: string; weight: number; italic: boolean; path: string; index: number };

export type InstalledFamily = {
  family: string;
  styles: InstalledStyle[];
  weights: number[];
  has_italic: boolean;
  is_variable: boolean;
  axes: Axis[];
  is_monospace: boolean;
  files: string[];
  user_installed: boolean;
};

export type CatalogFont = {
  id: string;
  family: string;
  category: string;
  subsets: string[];
  default_subset: string;
  weights: number[];
  styles: string[];
  variable: boolean;
  license: string;
  license_class: LicenseClass;
  source: string;
};

export type FontFile = { weight: number; style: string; subset: string; ttf: string | null; woff2: string | null };

export type FontDetails = CatalogFont & { version: string | null; files: FontFile[] };

export type FailedFile = { file_name: string; error: string };

export type InstallOutcome = {
  dir: string;
  installed: boolean;
  saved: string[];
  skipped: string[];
  failed: FailedFile[];
  cancelled: boolean;
  cache_refreshed: boolean;
  registered: number;
  needs_restart: boolean;
  coverage: "full" | "subset" | "";
};

export type InstallState = "downloading" | "saved" | "skipped" | "failed";

export type InstallProgress = {
  job_id: string;
  index: number;
  total: number;
  file_name: string;
  state: InstallState;
  error?: string;
};

export type FontRef =
  | { kind: "catalog"; id: string; family: string; category: string; variable: boolean; weights: number[]; styles: string[]; subset: string }
  | { kind: "installed"; family: string; category: string; variable: boolean; weights: number[]; styles: string[] };
